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

2026-10-13. `prune` now refuses a directory a live process holds (`tools/storage/src/inuse.rs`)
and defers to the overlay policy on any path the policy names or protects (`Authority` in
`tools/storage/src/prune.rs`), so `report` and `prune` give one answer about `~/.cargo-target`
instead of two. The policy was trimmed to what is on the machine; the reasons removed from it are
the section below.

### Three reclaimers, and only one of them runs

| who | removes | guard | size rule |
|---|---|---|---|
| `sjel storage apply` | every class with `apply = true` | the policy | **none** |
| `sjel storage prune` | target dirs, node_modules | canonicalise, git, in-use, policy | none |
| `capabilities/cargo-trim` (overlay, launchd, 1d) | `~/.cargo-target/debug` | its own `pgrep -xq cargo/rustc` | 50 GB cap |

`cargo-trim` is the only one that runs unattended, and it exists because `apply` is size-blind —
its own `service.toml` says a daily apply "would also clear the browser payloads and app caches
every day". So the one reclaim that happens carries its own guard, its own `du`, its own cap, and
its own knowledge that `release/` is live: everything the tool already has, rebuilt in 40 lines of
shell. Either the tool absorbs it — a cap per class, and `apply` becomes schedulable — or the
policy should stop presenting itself as the reclaim surface.

### No class has a size axis

The policy expresses *may I* (`apply`, `protected`) and never *is it too big*. Every incident that
built this tool was about size — 46 GB of cache, 149 GB of target, 130 GB of debug — and the one
size threshold, `class_flag_gb = 20`, fires on `rust-shared-target` most days and nothing acts on
it on purpose (`tools/sjel-cli/src/host_watch/pure.rs`). That axis is what would make reclaim
schedulable rather than a thing a person decides.

### `discover` stops at `$HOME`, and nothing runs it

79 GB of the used disk still has no instrument: `/opt/homebrew` 17.0, `/Applications` 13.4,
`/private/var` 8.7 (measured 2026-10-13). `discover` is the verb built for exactly this and has no
caller, so `--root <path>` and `--include-system` would widen a reading nobody takes. The Google
Drive `.tmp` deadlock already makes every figure a lower bound.

### 192 GB the policy does not name

Named 49.0 GB against 192.5 GB unattributed (2026-10-13, after the trim). Naming
`~/.ollama/models` (17.7 GB, in active use — report-only, not a reclaim target), iCloud and
`~/Music` (protected), `/opt/homebrew` and `/private/var/folders` (report-only), and a
developer-caches class (~5 GB across `~/.npm/_cacache`, `_npx`, `~/.local/pipx` and
`~/.codex/packages`, each with an owning cleanup verb) would take named to roughly 100 GB. Overlay
work, not a crate change.

### No class has a growth curve

`storage_samples` holds `named_bytes` and `unattributed_bytes`, and `storage_absent` holds which
names were gone; neither holds a per-class size, so a class that doubled is invisible. The sampler
already runs hourly, so this waits on nothing.

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

`ISA.md` 1,090 lines, `ISA-archive.md` 952, `CONTRIBUTING.md` 1,065, `dashboard/README.md` 781.

### Giant pages

`routes/travel/+page.svelte` 4,197 lines and `routes/feed/[id]/+page.svelte` 2,918. Split each the next time it is touched, as the interior page was in 83a5837b.
