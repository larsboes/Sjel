# Data: what comes in, in what shape, and what adapts

Every file has one owner and one way in. Facts have one shape wherever they are stored. The
models and keys the machine depends on are visible from one place. After Lane A's restructure in
[`PLAN.md`](../../PLAN.md).

## 1. Import

Measured 2026-10-23 with `sjel-import plan /Volumes/Extreme/archive` (25,348 entries) and the
documents library (7,154 records):

| The router reports | What is there |
|---|---|
| 643 documents | All already in Sjel |
| 5,788 media | 5,297 are resources inside 12 Swift Playgrounds bundles. About 490 personal photos and audio. |
| 3,009 notes | 2,074 LifeOS and Claude agent state. 237 Notability `.note` files. About 700 other `.md` and `.txt`. |
| 0 finance | No bank-statement CSV |
| 5,888 text records in documents | 2,149 copies of vault notes, 3,127 agent state. None filed. |

Decisions (2026-10-23):

1. **Media: check, then queue.** The library stays on the two mirrored external drives. Import
   hashes a file and asks media through a read-only route whether a library holds it. A new file
   waits until a library drive is mounted. Nothing is copied to the laptop.
2. **The vault keeps knowledge only** (2026-10-20,
   [projects README](../../capabilities/projects/README.md#why-this-shape-sjel-owns-projects-tasks-and-habits)).
   Finance, Items, Interior, People, Events, Media, Reflections and Journal have moved. Open:
   Documents, Focus and Personal. Then three follow-ups: `Knowledge/` cites Media sources with 140
   wikilinks Obsidian no longer resolves (the store paths are unchanged, `Atlas/Media/X.md`).
   Comms keeps saved feed items in its store; its former vault writeback has been removed. The
   vault bases and MOCs for the moved folders go, and the vault
   rules shrink into the `obsidian` skill. Decided 2026-10-28: all non-knowledge material,
   including `Archive/`, leaves the vault. Preserve unsorted material in Sjel's archive for
   later import into its owning capabilities; retain source files until each batch and its
   attachments are verified. Done when the vault holds only knowledge and its required Bases
   and assets, and all other material is preserved outside the vault.
   `Atlas/Personal/Archive/Schule` (139 notes, 1,279 files): notes to `projects`, PDFs and scans
   to `documents`.
3. **The journal gets its own capability, later.** Its 353 records, with habits and daily
   metrics, move out of the projects store.
4. **Old text records are archived with their bytes kept,** after their notes have an owner.
5. **Router fixes:** skip `.playgroundbook`, `.playgroundmodule`, `.swiftpm` and agent-state
   folders. Route `.key` (Keynote) to documents, and change documents to accept it.
6. **Finance** keeps its route. Its import contract waits for the first real bank export.

Open:

- **Obsidian cutover.** The new local vault is configured and `scripts/vault contract` passes at
  `Knowledge/Meta/Vault/Vault-Contract.md`. Every file of the old vault is accounted for:
  `tools/vault-archive-intake --verify` matches the 3,091 non-knowledge files byte for byte, and
  `Knowledge/` matches except the added contract and the rewritten `Meta/Vault/Vault.md`. Not
  carried: `.git`, `.obsidian` and `.smart-env` internals, and 19 dangling plugin-backup
  symlinks. The vault and scouting services were rebuilt and restarted after the pointer change
  and report ready. The 171 links into archived material are plain text now, `Focus.base` and
  its four embeds are gone, and `vault bases --strict` passes. The vault is a private repository,
  `github.com/larsboes/Knowledge-Base`; a fresh clone matched all 1,225 tracked files. Open:
  - Review settings, `Knowledge.base`, links and embeds in Obsidian.
  - `vault links` counts links inside fenced code blocks, which Obsidian does not resolve. The
    three remaining newly dead links are the `Document.pdf` syntax examples in
    `Knowledge/Meta/Vault/Obsidian Embed Files.md`.
  - Inert old-vault paths remain in the overlay: `SETUP.md`, three `profile_path` entries in
    `comms-relevance-shadow.json` (no reader), the two disabled scouting sources, whose globs
    name folders that left, and the `knowledge-base` state mount in `machine.toml`, which still
    describes a homepi bare repository.

  Cutover is complete when the Obsidian review passes. Keep the iCloud copy unchanged as a
  historical state.
- **Every route has an owner that accepts it.** Media, finance and notes files are recorded
  `pending` because those capabilities have no import contract. Done when a run over a real mixed
  folder leaves no `pending` row, no file goes to two owners, and a rerun stores no bytes twice.
- **Import from the app.** The GPUI Import page runs plan, run and status. Done when an import of
  a real folder from the app matches `sjel-import status`.
- **Media `To Sort/`.** 229 files (53 GB) in `Library/To Sort/` have no reviewed destinations,
  because both structure drafts in the overlay target removed Inbox trees. The categories are a
  human decision. Then `organize` can act.

### Web clipping into Sjel

Decided 2026-10-28: preserve the Obsidian Web Clipper workflow and its full functionality while
saving directly into Sjel. Existing clippings outside `Knowledge/` are preserved in the unsorted
archive and imported separately; the knowledge vault is not the clipping inbox.

Prior art: [Obsidian Web Clipper](https://github.com/obsidianmd/obsidian-clipper) is MIT-licensed
source, excluding its trademarks and marketing assets. Its documented features include page and
selection capture, persistent highlights, Reader, templates, variables, filters, logic and the
optional Interpreter. `comms` already accepts captures and `/ingest` accepts submitted content,
title, author and client provenance. Neither interface establishes full clipper parity. Defuddle
is already recorded in `upstreams.toml`. `capabilities/comms/axon-clip` already captures a rendered
page, selection or link into `/ingest` on Chromium browsers, with a popup, shortcuts and context
menus. It does not provide the upstream reader, highlights or templates. The extension stores its
configured bearer secret in browser-local storage; a replacement must review that credential
boundary rather than assume it satisfies the current keychain policy. The clipper is recorded as
`watch` in `upstreams.toml`; audit it before adopting code.

Initial upstream inspection: main resolved to `6d56d618b00bd970aa738d6a7a61edee27783e81`
on 2026-10-28. Its Chromium manifest requests all-URL access and registers page content scripts;
Sjel Clip currently requests `activeTab` and loopback hosts. Its `src/api.ts` exposes a clipping
and template API with a caller-supplied document parser and destination, a possible reuse seam.
Neither observation proves the API covers Reader, persistent highlights or Interpreter. Evaluate
permission scope, dependencies and the full parity matrix before choosing the integration.

#### Initial parity inventory

This inventory uses the upstream revision above, including `src/types/types.ts`, the three browser
manifests and the [upstream documentation](https://help.obsidian.md/web-clipper). It is not yet an
exhaustive feature or test inventory. The acceptance checks are required work, not passing tests.

| Surface | Required acceptance check |
|---|---|
| Capture | Save a rendered page, selection and URL; retain page metadata and user edits; reopen the saved item. Test authenticated and client-rendered pages, shortcuts, context menus and error feedback. |
| Reader | Open and close Reader from the supported entry points; retain content, images, headings, code and footnotes; restore font, layout, theme and custom CSS settings. |
| Highlighter | Select text and page elements, restore highlights on a later visit, search and export them, and test each highlight-to-content mode in Reader and on the live page. |
| Templates | Import, export, reorder, trigger and edit templates; compare variables, selectors, schema data, filters, conditionals, loops and properties against upstream fixtures. Verify selected text and highlight substitution. |
| Save behaviors | Preserve create, overwrite, prepend and append semantics, including specific and daily destinations. Define Sjel-owned destinations and conflict handling before implementation; do not silently turn an overwrite into a new capture. Preserve file and clipboard export. |
| Interpreter | Run prompt variables only when selected by the configured behavior; preserve context and filter semantics and provider/model configuration, with Sjel's data-class and credential rules. Verify both local and configured remote models. |
| Browser and device | Verify each supported browser build independently. The pinned Chrome manifest requests side panel and declarative network rules; Firefox requests web request blocking; Safari requests native messaging. All three declare broad web hosts. Assess each permission, installation and local transport rather than assuming a shared manifest or a working iPhone path. |
| Settings and data | Import existing templates and highlights without losing their originals. Preserve highlighter and reader settings and history where possible; verify migration, offline queue recovery, duplicates, and backup restore. |

Upstream exposes `src/api.ts` for extraction and template compilation, while separate Reader,
highlighter, settings and browser code implement other behavior. Its tests include API, extraction,
filters, highlighter and template integration tests; passing those alone would not establish
device-level parity. Its template type also includes `overwrite`, which requires an explicit
destination and conflict contract. Inspect the remaining source, tests and installed clipper data
before declaring this matrix exhaustive or choosing between an adapter and a fork.

The present `/ingest` route is not the full-clip persistence contract. It identifies feed items
by canonical URL, extracts submitted HTML before assigning `raw_content`, updates the existing
raw extraction and transcript when a later deterministic capture wins, and starts background
enrichment after storage (`comms/src/media.rs`, `store/feed.rs`, `server/feed.rs`). The server's
JSON body limit is 2 MiB (`server/main.rs`), and a newly ingested item starts undeclared at `c1`
(`store.rs`). Consequently, a retry and a second distinct highlight from the same page need
different identities, and the submitted DOM and user annotations need durable storage before any
feed projection or enrichment. Large captures need a bounded attachment path and explicit failure
behavior; raising the global request limit alone does not provide it. Classification and egress
must be resolved before enabling automatic processing for browser-provided material.

1. **Inventory and parity specification.** Pin an upstream revision and enumerate its features,
   browser permissions, settings, storage, template formats and tests, including Safari on macOS
   and iOS. Define a parity matrix against real workflows and existing user templates and
   highlights. Done when each upstream behavior has a Sjel acceptance test or a named open
   decision; do not claim full parity from a selected feature list.
2. **Capture contract.** Define an owned `comms` contract for page content, original URL and
   metadata, highlights, user edits, templates, attachments and provenance. Preserve original
   bytes where available, version the payload, make retries idempotent and expose failures.
   Review data classification, authorization and local-only handling before browser access.
   Done when retries, attachment recovery and partial failures are tested without duplicate or
   lost captures. Existing capture and ingest routes remain usable. In progress: an unexposed
   `comms` store primitive holds versioned JSON under a client retry key, refuses conflicting
   reuse and records `c2`; it reads a stored clip by its key and does not fetch or enrich. Inline
   JSON is bounded at 2 MiB. It has no browser route, attachment contract or recovery UI.
   `cargo check --locked -p sjel-comms --tests` and the two focused
   storage tests passed; the tests required a scratch Cargo target because cleanup of temporary
   files under the shared target was denied. A prototype HTTP route passed a local authentication
   and retry test, but was removed: `sjel-status` proxies the Comms path, so a loopback bind alone
   does not prove that `c2` content stays on the host. Define and test a host-only transport and
   agent access policy before exposing a clip route; do not assume origin or client-supplied
   forwarding headers prove locality.
3. **Browser integration.** Evaluate a maintained upstream fork against a minimal adapter after
   the inventory; reuse upstream extraction, Reader and highlighting where feasible. Preserve
   existing template import/export, URL and schema triggers, variables, filters, logic, and
   create/append behavior through explicit Sjel destinations. Preserve highlight persistence,
   search/export, selection and keyboard workflows. Queue safely when Sjel is unavailable.
   Done when the parity matrix passes on each supported browser and the original workflow can
   save and reopen a capture without Obsidian.
4. **Interpreter and Sjel use.** Preserve prompt variables, configurable context, provider/model
   choices and explicit interpretation; connect to Sjel inference only under its data boundaries.
   Keep source material and user edits separate from generated results. Add review and routing to
   owning capabilities with provenance, and allow knowledge to be linked into the vault when
   warranted. Done when captures can be reviewed and routed without a second canonical copy.
5. **Migration and end-to-end verification.** Import existing clipper templates and highlights
   where accessible, and preserve historical clippings and attachments before routing them.
   Test offline/retry, duplicate submissions, large pages, images, authenticated pages, browser
   restarts, and backup/restore on Mac and iPhone. Done when a real clip survives capture,
   restart, review and restore, and no existing clipping is lost.

#### Delivery order

1. Complete the upstream inventory against real templates, highlights and settings. Give every
   upstream behavior a parity test or a named open decision.
2. Decide supported browser and device targets and ownership (maintained fork or adapter). Keep
   the current Chromium capture extension working while the replacement is built.
3. Specify the `comms` record, host-only authorization, attachment storage, retry identity,
   duplicate handling, backup and restore. Store source bytes, extracted text, highlights, edits
   and Interpreter output separately with provenance. Prove browser content cannot leave the host
   except through an explicit Sjel workflow.
4. Port or adapt capture, Reader, persistent highlights, template editor and runtime, variables,
   filters, logic, destinations, settings, import/export and offline queue. Add parity checks from
   the inventory at each stage; a first capture release is not full support.
5. Add Interpreter through Sjel inference with explicit invocation, visible provider/model, and
   preserved source versus generated output. Apply existing data-class and egress rules.
6. Migrate user templates, highlights, settings and historical clippings with a dry run, counts,
   idempotent import and restore rehearsal. Keep originals until parity and recovery checks pass.
7. Run the browser/device matrix, including authenticated and client-rendered pages, offline
   recovery, browser restart, attachment failure, duplicate retries and backup restore. Retire
   Obsidian as a clipping destination only after this passes.

Open decisions: the first supported browser and device, exact Sjel mapping for existing-note and
daily-note template actions, and whether to maintain a fork or a narrow adapter. Resolve these
after the parity inventory; do not silently drop functionality to make the first release smaller.

## 2. Formats

- **PDF rule** (2026-10-23). Official documents (contracts, invoices, certificates, tax notices)
  stay PDF-first. For any other document, the extracted data in the owning capability is what
  Sjel uses. The original stays archived and hash-verified, and is never removed.
- Convert the 237 Notability and GoodNotes `.note` files to PDF, then OCR and enrich them.
- Align formats across capabilities, so one fact has one shape.

## 3. Settings surface for inference and keys

Decided 2026-10-24: the backends, models and keys are readable and provable from one section of
the GPUI Settings tab. No new client, page or CLI verb set.

- Why: on 2026-10-24 `tools/graphify.sh` ran a full pass on a model retired since 2026-08-07. The
  only record of that model was a backend field that no tool read.
- Owner: `capabilities/settings`, read and probe first, writes later. It reads `inference.json`
  through `libs/inference` and re-implements none of it.
- **What works today.** Settings › Accounts stores a provider key in the OS keychain as
  `inference-<provider>` (`apps/gpui/core/src/shell.rs`, `store_provider_key`), for OpenAI,
  DeepSeek, OpenCode Go, NVIDIA NIM and Gemini. A backend names that item with
  `api_key_credential_id`. `libs/inference` and `tools/model-check` resolve a key through one
  function, `backend_api_key`: keychain first, then a key file. `tools/model-check --role <role>
  --probe` sends one request to one role. Checked 2026-10-25: the `deepseek` and `gemini` backends
  answer from their keychain items alone.
- **Model policy** (2026-10-25). Gemini: `gemini-3.5-flash-lite` for cheap bulk roles
  (`cloud_gemini_content_analysis`), the newest flash model for every other role
  (`gemini-3.8-flash` on 2026-10-25). DeepSeek: the flash model, `deepseek-flash` on DeepSeek's own
  API and `deepseek-ai/deepseek-v4.1-flash` on NVIDIA NIM. `tools/model-check` reports "newer
  available" when a newer flash model appears, and `tools/model-check --list <backend>` prints a
  backend's catalogue. NVIDIA's free content-analysis role runs
  `nvidia/nemotron-3.5-lightning-30b-a3b`, because the older Nemotron ids are listed but answer
  404.
- **One key path.** Settings › Accounts writes the keychain, and that is the path for every
  provider it lists (OpenAI, DeepSeek, OpenCode Go, NVIDIA NIM, Gemini). The older path,
  Vaultwarden into a key file through `tools/materialize-inference-key`, still feeds `nvidia-nim`,
  `opencode-go`, `cohere`, `groq`, `cloudflare-workers-ai` and `assistant-openai`. `omlx` reads
  the local server's own key from oMLX's settings and stays as it is. Next:
  enter the NVIDIA key in Settings, then remove `api_key_file` from each backend that has a
  keychain item, delete its key file, and add Settings entries for the providers that have none.
  The Gemini key file was removed on 2026-10-25.
- **Next.** The settings section shows key presence only. It should show, per backend, whether its
  model answers, and offer the one-role probe as a button.
- Shows, in order: each backend with its model, whether the model answers, and whether its key is
  present. Each role with its backend and model. Then the tool-level selection no role names
  (`config/graphify.env`).
- No writes in the first delivery. A write needs a provider-side check and an undo path.
- Built with the gates from the first commit: a `const ROUTES` manifest, a free `service.toml`
  port, regenerated `ARCHITECTURE.md` and `self.json`.
- Known obstruction: secrets-guard refuses the tracked `tools/graphify.env.example` by filename.
  It needs its own issue.
- Done when one authenticated read names every backend with its model and key state, every role
  with its backend and model, and the tool-level selection, and a probe proves each model answers.
  It fails if a tracked artifact names a model or key file that the surface does not report, or if
  its verdict for a backend disagrees with `tools/model-check`.

## 4. Adaptive data

Features should adapt to what the user actually does. Not designed yet. Inputs that
exist:

- `capabilities/operator-profile` and `capabilities/traveler`: preferences and derived patterns.
  Profile proposals are served on `GET /api/profile/proposals`, and pseudonymous companion
  patterns on `GET /api/profile/derived`. Open: the offered-versus-chosen half. 28 `option_set`
  items exist on one plan, and one plan is one observation, not a preference.
- The dashboard fallback log ([`gpui.md`](gpui.md#3-what-retiring-the-dashboard-still-takes)) as
  a model for measuring use before changing a surface.

A proposal comes after Lane A, and it starts from measured use, not from guesses.

## 5. Domain backlog

Open items that belong to one capability and to no priority yet.

- **Documents.**
  - External backup: see [`storage.md`](storage.md#5-backups).
  - Enrichment: folder intake and polling exist. Source identities, mail attachment intake,
    source-linked insights and class-gated model enrichment do not. Done with a frozen German and
    English corpus, declared acceptance criteria, recorded results and an idempotent mail replay.
    No `c2` document reaches cloud inference, and no `c3` content enters a prompt.
  - Mixed scans: approved partitions already create child PDFs with exact page ranges. Broader
    boundary detection and a measured evaluation on annotated multi-document scans remain.
- **Mail event extraction.** Explicit invitation date, time range and location labels establish the
  event frame; the local model extracts grounded programme items from invitation text. Other mail
  uses the same model with grounded evidence. Open: measure extraction against a reviewed English
  and German mail corpus. Done when replayed invitations retain timed events and sourced programme
  details while unsupported dates and agenda items remain rejected.
- **Mail classification labels.** The harness exported 121 fixtures with 0 labels
  (`<overlay>/config/comms-mail-stream-shadow.json`). Labels are hand judgements, written before
  any model output is read. The stream switch needs model agreement of at least control plus 10
  points and a false-eviction rate of at most 2%. Urgency has its own gate.
- **Finance investments.** `instruments` holds 13 records and `allocations` is empty. Open: the
  target cohort (active allocations sum to 10,000 bp ±50), tickers where known (a wrong ticker
  prices the wrong company silently), and the `finance-prices` launchd unit.
- **Trip numbers.** A budget on a plan and a trip id on a transaction are data entry. The finance
  per-trip route answers 200 with five zeros for an untagged trip.
