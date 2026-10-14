# Plan

Ideas and open questions. A decision that has shipped moves into the code and leaves this file.
The contract of each unit stays in its `ISA.md`.

## Capture: what is still open

Shipped 2026-10-13: `comms` `/captures`, `sjel capture`, and the dashboard page `/inbox`.

### Capture from the phone

The phone reaches Sjel only through the LAN listener, and that listener admits only requests signed with a paired device's key (`libs/sjel-server/src/lan.rs`). An iOS Shortcut cannot sign a request, so it cannot post to `/captures`. Decided 2026-10-13: an iOS share extension in the phone app, which signs with the paired key. A shared-token endpoint was rejected because it opens the signed-only listener.

### `#go` has no destination

It stays in the inbox. Decided 2026-10-13: it goes to `places`, which needs a write path for a place from a link or a note.

### Model suggestions

Domain rules suggest a tag (`dashboard/src/lib/capture.ts`). The model does not yet suggest anything for what the rules leave.

### Prices a page does not declare

`inventory wunsch` reads a price only from meta tags. Uniqlo declares none and has no JSON-LD (measured 2026-10-13). Its price is only in its own app state, as `"prices":{"base":{...,"value":39.9}}`. Open: a per-shop reader, or a price typed in at triage.

### Groups of wishes

A group with one constraint for all its items ("travel-friendly, fewer but better") has no shape in inventory. For now it is an untagged note in the inbox.

## Storage: what is still open

2026-10-13. The crate has **one reporting surface and one reclaim surface**. `report`,
`discover`, `report --where` and `target` read; `reclaim` is the only thing that removes, with
`apply` and `prune` kept as spellings of it so a finding already written that says `--apply`
still resolves. Every removal goes through one guard (`guard.rs`): canonicalise, scope, git, the
policy's protection and ownership, then the live-process probe. The policy gained a size axis
(`cap_gb`, `older_than_days`), which is what let `capabilities/cargo-trim` stop being a shell
script with its own `du`, its own `pgrep` guard and its own 50 GB cap — it runs
`reclaim --scheduled` now.

`discover` took `--root <path>` and a caller, so the space the report could only total is now
ranked. `--root /System/Volumes/Data` covers the scope `unattributed` does; `--root
/opt/homebrew` (17.0 GB, 0 named), `/Applications` and `/private/var` answer on their own.
`--root /` is refused, because the Data volume is reached through a firmlink rather than a mount
point and `du -x /` counts it inside `/System`. The walk costs 35 s under `$HOME` and 45 s over
the volume, so it runs on demand and never in the hourly report.

Shipped 2026-10-13: named paths in overlay policy brought named space to 156.2 GB; per-class growth curve shipped to `storage_classes` in `capabilities/sjel-status` and surfaced in dashboard.

## Storage declarations that were retired

Removed from the overlay's `config/storage-policy.toml` on 2026-10-13 because their paths were
gone, and a dead entry costs a line in every report while teaching the next reader that the file is
stale. The reasons stay here, because a reason is what a scanner cannot infer. Re-add a class only
with a measurement that the thing is back.

**oMLX KV cache (`~/.omlx/cache`).** Retired: `~/.omlx` is absent, no `omlx-server` process runs,
nothing listens on :8000, and `capabilities/foundation-models/service.toml` records oMLX as retired
with apfel serving the light role. The part worth keeping: **if oMLX returns, do not move its cache
to the external SSD** — 230 MB blocks over USB cost more on every cache hit than the internal NVMe
read they replace. It grew unbounded because `settings.json` has `hot_cache_max_size = "0"` and
`ssd_cache_max_size = "auto"`; cap it there rather than clearing it on a schedule.

**`postgres` and `omlx-server` as expected services.** Both were `[[expected_service]]` and both
are retired. The report printed `ABSENT` beside each while a note underneath contradicted the line
above it — a report that finds something on every run and is right every time is one nobody reads.

**Playwright / puppeteer payloads and Chrome's on-device models.** `~/.cache/puppeteer`,
`~/Library/Caches/ms-playwright{,-go}` (about 5 GB if a project installs one) and Chrome's
`OptGuideOnDeviceModel` and `screen_ai` (about 4 GB). All absent on 2026-10-13, and **these are
armed rather than dead**: the tool that would fill them still exists, so if one comes back its
bytes land in `unattributed` until a class names it — the exact failure this tool was built to
catch. Re-add a class when `discover` shows one.

**Per-crate Cargo target dirs (`~/Developer/Sjel/{capabilities,libs}/*/target`).** Absent, and
verified 2026-08-08 that they stay absent: parking `capabilities/comms/target` and rebuilding wrote
only to the workspace root. **If one reappears, that is the signal something builds outside the
workspace** — investigate rather than re-delete it.

**`~/Library/Caches/bazel`, and the absent protected path.** No `bazel-bin` or `bazel-out` exists
anywhere in the checkout and cargo is the build system; bazelisk is installed but produces nothing
here. The protected path `~/Library/Application Support/com.apple.container` was absent too — it
guarded Axon's store, and a missing database is a question for its owner, which is what was
actually open there.

## Code structure: what is still open

Measured 2026-10-13. Decided that day: German identifiers move to English.

### German identifiers to English

About 700 identifiers in `interior` and `inventory` are German: `preis_cent`, `groesse`, `farbe`, `prioritaet`, `kaufreihenfolge`, `monatssaldo`, `Fehler`. Many are JSON fields and database columns, so the move needs a store migration, a change to the phone outbox's queued payloads (`dashboard/src-tauri/src/sync.rs`), and an import that still reads the old TOML keys. New code is English from now on.

### `dashboard/src/lib/api.ts` is 4,700 lines

All capability clients are in one file. Candidate: one file per capability under `lib/api/`.

### Axon names

1,823 occurrences in 518 files (`axon.toml`, `axonStatus`, `axon-clip`, `AxonGlance`). The config file names need a fallback read for machines that still have the old name.

### Root documents

`ISA.md` 1,090 lines, `ISA-archive.md` 952, `CONTRIBUTING.md` 1,065, `dashboard/README.md` 781. Shipped 2026-10-14: introduced clean modular `docs/` technical documentation structure (Vector / Rust Reference standard) across architecture, capabilities, and operations, with in-app `/docs` viewer route in dashboard via `virtual:sjel-docs`. Decouples historical narrative from current interface contracts.

### Giant pages

`routes/travel/+page.svelte` 4,197 lines and `routes/feed/[id]/+page.svelte` 2,918. Split each the next time it is touched, as the interior page was in 83a5837b.
