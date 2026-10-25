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
   `comms` still writes saved items as source notes into the vault's `Resources/Sources/`
   (`comms/src/projection.rs`). The vault bases and MOCs for the moved folders go, and the vault
   rules shrink into the `obsidian` skill. Done when the vault holds no non-knowledge note outside
   `Archive/`.
   `Atlas/Personal/Archive/Schule` (139 notes, 1,279 files): notes to `projects`, PDFs and scans
   to `documents`.
3. **The journal gets its own capability, later.** Its 353 records, with habits and daily
   metrics, move out of the projects store.
4. **Old text records are archived with their bytes kept,** after their notes have an owner.
5. **Router fixes:** skip `.playgroundbook`, `.playgroundmodule`, `.swiftpm` and agent-state
   folders. Route `.key` (Keynote) to documents, and change documents to accept it.
6. **Finance** keeps its route. Its import contract waits for the first real bank export.

Open:

- **Every route has an owner that accepts it.** Media, finance and notes files are recorded
  `pending` because those capabilities have no import contract. Done when a run over a real mixed
  folder leaves no `pending` row, no file goes to two owners, and a rerun stores no bytes twice.
- **Import from the app.** The GPUI Import page runs plan, run and status. Done when an import of
  a real folder from the app matches `sjel-import status`.
- **Media `To Sort/`.** 229 files (53 GB) in `Library/To Sort/` have no reviewed destinations,
  because both structure drafts in the overlay target removed Inbox trees. The categories are a
  human decision. Then `organize` can act.

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
- **Mail classification labels.** The harness exported 121 fixtures with 0 labels
  (`<overlay>/config/comms-mail-stream-shadow.json`). Labels are hand judgements, written before
  any model output is read. The stream switch needs model agreement of at least control plus 10
  points and a false-eviction rate of at most 2%. Urgency has its own gate.
- **Finance investments.** `instruments` holds 13 records and `allocations` is empty. Open: the
  target cohort (active allocations sum to 10,000 bp ±50), tickers where known (a wrong ticker
  prices the wrong company silently), and the `finance-prices` launchd unit.
- **Trip numbers.** A budget on a plan and a trip id on a transaction are data entry. The finance
  per-trip route answers 200 with five zeros for an untagged trip.
