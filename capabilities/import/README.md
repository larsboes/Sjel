# import

Sorts a folder to the capability that owns each file and records every file's outcome. It
hands each file over through the owner's HTTP contract.
It only reads source files, and it never moves, renames or removes one.

```bash
sjel-import plan /Volumes/Extreme/archive          # sort only; changes nothing
sjel-import plan /Volumes/Extreme/archive --list   # one line per file with its route
sjel-import run /Volumes/Extreme/archive           # hand over and record
sjel-import status /Volumes/Extreme/archive        # what the ledger holds for that folder
sjel-import serve                                  # the operator API (service.toml)
```

The Sjel app's Import page (`g u`, `apps/gpui/src/import.rs`) drives the same steps through the
API: choose a folder, see the plan, start a run, watch it, and read the ledger. The API admits
the deployment credential only, never an agent token, because a plan reads any folder the
caller names. One run is active at a time.

**Why a capability of its own:** before it, each capability had its own way in with its own
rules (`documents inbox`, `media ingest` of a staged export, finance's bank import). The
operator had to sort a folder by hand before any of them could take it. Worse, the documents
folder intake accepted `.md`, `.txt` and `.csv`, so a dump of agent state became 5,888 records
in the document library. Each file type now has one owner, and the router decides.

## Routes

| Route | Takes | Handover |
|---|---|---|
| `documents` | PDF, Office, iWork, ODF, RTF, EPUB | `GET /api/documents?sha256=` first, then `POST /api/documents` |
| `media` | photos, video, audio | none yet: recorded `pending` |
| `finance` | a CSV whose header has two or more bank-statement words | none yet: recorded `pending` |
| `notes` | `.md`, `.markdown`, `.txt`, Notability `.note`, GoodNotes `.goodnotes` | none yet: recorded `pending` |
| `skip` | software, caches, bundles, hidden files, unknown types | not handed over, with the reason in the ledger |

The rules are in `src/route.rs`. The router records a software folder (`node_modules`, `plugins`,
`skills`, an `.app` bundle and others) once and does not enter it.

## Outcomes

| Outcome | Meaning | Next run |
|---|---|---|
| `imported` | the owner stored new bytes | not read again while size and mtime stay the same |
| `present` | the owner already held the same bytes, under this or another filename | same |
| `skipped` | not personal data, or the owner refused it (for example a sensitive filename) | same |
| `pending` | sorted, but the owner has no import contract yet | checked again |
| `failed` | the handover failed | retried |

The ledger is the table `import_files` in the shared database, keyed by path. `status` answers
"is everything from this folder in Sjel" without reading the folder again.

## Decisions (2026-10-23)

1. **A capability, not a client feature.** The GPUI app, the CLI and agents need the same
   sorting. Cross-capability use goes through HTTP contracts, so this crate imports no other
   capability's code.
2. **Best guess, fix later.** The operator chose immediate routing over a review queue for
   uncertain files. A wrong route shows in `status`, and the fix goes into `src/route.rs`.
3. **One record per set of bytes.** documents keys a record by bytes plus filename, so the
   same file under a second name made a second record. The router asks documents for the
   SHA-256 first and records `present` instead of uploading.
4. **documents folder intake no longer takes `.md`, `.txt` or `.csv`.** The 5,888 existing
   text records stay. Their cleanup is a separate decision.

Open work: `ISA.md`, section "import". The order and the decisions for the remaining routes:
[`PLAN.md`, track 2](../../PLAN.md#track-2-data-into-sjel).
