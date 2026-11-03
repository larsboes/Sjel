# GPUI: one native app

The native GPUI client ([`apps/gpui`](../../apps/gpui/README.md)) is the one Sjel app on macOS and
iOS, with Linux, Windows and Android later. The SvelteKit and Tauri dashboard (`dashboard/`) is
the second consumer, and it retires (decided 2026-10-22). Port each page, then improve it in GPUI.
A rule that both clients share goes into a capability or `apps/gpui/core`, never only into
`dashboard/`. No new feature work goes into the dashboard.

Lane A in [`PLAN.md`](../../PLAN.md). Lane B, the agent, has priority when the two compete. The agent seat (Work Package 22) is in
[`agent.md`](agent.md), RoomPlan and the other scanners are in [`scanning.md`](scanning.md), and
the phone's store and sync are in [`storage.md`](storage.md).

## 1. Route matrix

The matrix tracks the 25 main dashboard routes. `dashboard/src/routes` holds 36 pages, and §3
names the rest.

| # | Route | Status | Mac | iPhone | Owner |
|---|---|---|---|---|---|
| 1 | `/` | Ported | `Home` + `ContextPage` | `Today` tab | shell |
| 2 | `/context` | Ported (one page with `/`) | `Home` + `ContextPage` | `Today` tab | `projects`, `calendar` |
| 3 | `/calendar` | Ported | `CalendarPage` | tab, reads and writes the replica | `calendar` |
| 4 | `/projects` | Ported | `Projects` | behind `Today` | `projects` |
| 5 | `/documents` | Ported | `Documents` | planned reader | `documents` |
| 6 | `/inbox` | Ported | `Inbox` | tab, capture only | `comms` |
| 7 | `/feed` | Ported | `Feed`: Reading, Review, Library, Mail, Discover | planned | `comms`, `scouting` |
| 8 | `/travel` | Partial | `Travel` | tab | `trips`, `traveler`, `transit` |
| 9 | `/map` | Partial | `Travel::Map` | native map | `trips`, `places` |
| 10 | `/finance` | Ported | `FinancePage` | planned | `finance` |
| 11 | `/inventory` | Partial | `InventoryPage`: state history and layout impact review missing | planned | `inventory` |
| 12 | `/wishlist` | Partial | `InventoryPage` tab | planned | `inventory` |
| 13 | `/interior` | Ported | `InventoryPage` tab | Interior tab, see [`scanning.md`](scanning.md) | `inventory`, `interior` |
| 14 | `/people` | Ported | `PeoplePage` | tab, People and Places, read-only | `people-registry`, `entities`, `places` |
| 15 | `/software` | Ported | `Software` | Mac only | `software` |
| 16 | `/backup` | Ported | `BackupPage` | status only | `backup` |
| 17 | `/capabilities` | Ported | `Capabilities` | status only | shell |
| 18 | `/settings` | Ported | `Settings` | behind `Today` | local state |
| 19 | `/systems` | Ported | `SystemsPage` | none | `sjel-status`, `macmon` |
| 20 | `/packs` | Ported | `PacksPage` | none | `tools/harnesses` |
| 21 | `/self` | Ported | `SelfPage` | none | `self.json` |
| 22 | `/cv` | Ported | `CvPage` | none | `cv` CLI |
| 23 | `/media` | Ported | `MediaPage` | none | `media` |
| 24 | `/research` | Ported | `ResearchPage` | none | `research/`, `upstreams.toml` |
| 25 | `/setup` | Ported | `SetupPage` | QR scanner | `devices`, `sjel-status` |

## 2. Phone priority areas

Priority 1 in [`PLAN.md`](../../PLAN.md): the phone is the daily tool for the agent, other agents,
travel, documents, mail and calendar. The agent and the terminal are in [`agent.md`](agent.md).

**Tab bar.** The bar holds five items at most (`mobile_tab_bar` in `ui/src/nav.rs`, 64 px per item,
375 pt on the narrowest iPhone). Implemented 2026-10-28: `Today | Mail | Calendar | Travel | Agent`.
The Agent tab triggers the agent seat. Inbox capture lives as a quick-capture bar on Today.
Documents and People open directly from Today 1-tap shortcuts, finder, and links.

| Area | Status | Notes |
|---|---|---|
| Documents | Mac only / mobile shortcut | PDFKit in `src/`. 1-tap shortcut on Today. |
| Mail | Shared compact + mobile drawer | `ui/src/mail.rs` density toggle (Compact/Desktop/Comfortable), mobile 44pt touch targets, expandable message snippet drawer with direct triage (Dismiss, Waiting, Archive, Trash, Categorize). |
| Calendar | Tab (replica + copy) | 24-hour day time grid with live red now-line, hour rail tapping to open Composer, Google & iCloud sync status mark. |
| Travel | Tab | Grouped Day cards, transfer connection buffer badges, one-tap booking code copy chips, and offline-ready marks. |
| Agent | Tab + Edge Seat | Agent session tier picker (`sjel | project | account`), policy block cards with rule ID & single override. |
| Capture | Today quick-capture bar | Quick-capture bar + People & Places and Documents 1-tap shortcuts on Today landing page. |

### What may live on the phone

`sjel mirror build` deliberately leaves comms, documents and finance out of the phone's copy
(`tools/sjel-cli/src/mirror.rs`, `PHONE_SET`). Mail and documents on the phone follow three rules
(decided 2026-10-25):

1. **Online first.** Mail and documents are read live from the Mac through the signed device
   client.
2. **Offline only what the user chose.** A document the user marks "keep offline", and a mail the
   user opened, are kept in the app sandbox with iOS Data Protection
   (`NSFileProtectionComplete`). Nothing else from comms or documents is copied.
3. **No second writer.** Triage on the phone writes through the same `POST /comms/triage/bulk`
   route. Without a connection the action waits in the outbox and is shown as pending.

### Documents

1. Done: `Documents` lives in `ui/src/documents.rs` behind the `PdfRenderer` trait; Core Graphics
   (`src/pdf_kit.rs`) implements it on macOS. Open: iOS `PdfRenderer` wiring.
2. The phone lists documents from `GET /api/documents` with cursor paging (§7), and opens the
   hash-verified original at fit width with pinch zoom.
3. "Keep offline" stores the original and its metadata on the phone (rule 2).
4. Then [#245](https://github.com/larsboes/Sjel/issues/245) (filing and side-by-side compare),
   [#247](https://github.com/larsboes/Sjel/issues/247) (reviewable proposals) and
   [#246](https://github.com/larsboes/Sjel/issues/246) (durable duplicate and split decisions).

Done when a real multi-page PDF opens at fit width on the phone, a kept document opens with the
Mac unreachable, and the original's SHA-256 on the phone equals the library's.

### Mail

1. A phone tab over the shared Mail page in a compact density: the proposals grouped by category,
   one row per mail with sender, subject and age.
2. Opening a row shows the mail's text and its attachments. An attachment opens in the document
   reader.
3. Swipe actions map to the existing `MailAction`s: archive, trash, dismiss, waiting. Archive,
   trash and a class-raising category ask first, as on the Mac (`MailAction::confirmation`).
4. Offline: opened mails stay readable (rule 2), and an action waits in the outbox (rule 3).

Done when the phone triages ten mails with the Mac reachable and the Mac shows the same state, an
action taken offline applies once on reconnect, and an attachment opens in the reader.

### Calendar

1. A day view with a 24-hour time grid and a now line. The week view stays as it is.
2. Long-press and drag moves an event in 15-minute steps. The write goes through the replica, the
   same path as the New form.
3. A status mark shows the last successful Google and iCloud sync, and an error if one failed.

Done when a moved event shows the new time on the Mac after the next sync, and the mark matches
the Mac's last sync time.

### Travel

0. **A destination has a coordinate.** Shipped: `capabilities/trips/src/place_resolve.rs`
   resolves one on save (registry by exact name, then the geocoder) and stores it on the plan's
   origin, destinations and stages, so the map, climate, distance joins and the accommodation
   anchor all have one. Rule 2 below draws nothing for an upcoming trip without it: before this
   the live travel layer held eight trip-destination points, all `phase: past`, and no upcoming
   trip at all. Shipped (client wiring): GPUI calls `POST /trips/api/plans/:id/resolve-places`
   when a trip opens with unlocated stops, stamps the stored plan, and exposes "Locate stops" on
   the map overlay and trip page, plus "Refresh coordinates" for repairs. Still open: **the past
   plans still carry no coordinates** (several do, from an older import). Backfilling them means
   resolving names like `Valencia, Spain → Nice, France` and `Hamburg, Frankfurt`, which are two
   places in one string: a single geocoded point for those is worse than a null, so the backfill
   needs a decision about compound names before it runs.
1. **Itinerary timeline.** Shipped: one card per day with date header, day summary counts,
   transport modes with icons, and connection buffer badges (seamless same-hub vs required buffer,
   plus overnight layover notes). Day cards embed linked trip events and nearby calendar events
   with full start–end ranges, locations, and formatted markdown programme notes. Items render
   type badges, address mapping, and 1-tap booking code copy chips.
2. **Map.** Shipped: category pins for stays (🛏), stations (🚆), airports (✈) and places (📍).
   Interactive stop card on map overlay with stop name, category badge, address, dates, and "Open in
   Apple Maps" deep link. "Fit trip" control re-centers and fits all trip stops.
3. **Offline-ready mark.** Shipped: computed per trip via `check_offline_readiness()` based on
   cached details and tile coverage for bounds across zoom 10–14. Trip header shows `✓ Offline ready`
   when verified, or an offline readiness percentage with a "Download offline pack" action that
   asynchronously prefetches missing tiles and details. Trip library displays `✓ Offline` status
   for cached trips.
4. **Fare alerts.** Work Package 27, verified on a device.
5. **Edits on the phone.** Trip fields and bookings, through the Trips routes the Mac uses.

Done when an upcoming trip shows the offline-ready mark, then opens with its timeline, map and
booking PDFs in airplane mode.

### Capture

A share extension in the phone app sends a link, text or a file to Inbox. The listener admits only
requests signed with a paired device's key, and an iOS Shortcut cannot sign, so a shared-token
endpoint is not an option. The extension signs with the app's device key through a shared
keychain group.

Done when sharing a link from Safari with the Mac away puts it in the outbox, and it reaches the
Mac's Inbox once on reconnect.

**Open in the trip model.** A stage names a destination and a date, an item names a day, and a
plan's status is draft, saved or archived. Nothing says "this part is undecided, here are the
branches". A second trip planned this way shows whether the model needs it.

## 3. What retiring the dashboard still takes

The dashboard, its Tauri shell and the Tauri plugins are deleted (2026-11-03). GPUI has no browser
handoff: a path no page answers opens the page of its first segment. No rule lives only in
TypeScript: `merge-candidate` is `capabilities/entities/src/duplicates.rs`, `who-is-around` is
`core/src/travel/around.rs`, `mail-model-proposal` is `ModelVerdict::proposes_change` in
`core/src/mail.rs`. Foundation Models in the Tauri app is retired.

The former gates are now open checks. None of them ran:

1. Work Packages 23 (People writes) and 24 (Finance import) pass the manual acceptance test.
2. The new views: People → Duplicates, a trip's "Who is around", the mail model verdict with
   Accept, and Retry or Cancel on a stopped Gmail action.
3. Fare alerts (Work Package 27) on a device, and the legacy RoomPlan drafts on a phone that holds
   real Tauri scans. Check whether the Tauri iOS build (`com.larsboes.sjel`) is still installed,
   and remove it.

Open work:

- `/feed/[id]`: queueing and running a cloud job (`/comms/content/cloud-jobs`), and lowering a
  data class, which needs a rationale.
- `/inventory`: state history and layout impact review.
- The stay ranking board an unfinished session left in the dashboard. It held one trip's hotels
  as constants and kept its ranking in memory. A port stores the candidates as `stay` items and
  needs an item update route in `trips` to hold a rank.
- Screenshots of the app for the public landing page ([`restructure.md`](restructure.md#4-the-public-page)).

## 4. Work packages

### Done

| WP | What | Commit |
|---|---|---|
| 0 | Systems and host telemetry (`/systems`) | `fbd7adea` |
| 1 | Packs and harness deployments (`/packs`) | `9a23cff1` |
| 2 | Self-model (`/self`) | `8f05e9a0` |
| 3 | CV and applications (`/cv`) | `9735e371` |
| 4 | Media ingest ledger (`/media`) | `2f0f92f2` |
| 5 | Research and decision models (`/research`) | `12ce8e75` |
| 6 | Setup and QR pairing (`/setup`) | `106a266d` |
| 7 | Phone Calendar tab, off the Mac's read-only copy | `2d867ad0` |
| 8 | Calendar joins the replica, the phone writes | `b5cfacf6` |
| 9 | Phone People and Places | `894eed0e` |
| 10 | Phone opens on `Today` | `a1cb5218` |
| 11 | Outbox goes around a refused write (#244) | `9ba867c3` |
| 12 | Phone Inbox capture | `e505d6af` |
| 13 | Mac captures join the phone's inbox | `aaf300e7` |
| 14 | Soundscape on the phone | `41e333e7` |
| 15 | Phone local notifications | `b51839ef` |
| 16 | One right edge with seats | `0ba92b64` |
| 17 | Trips written on the Mac | `68dd362c` |
| 18 | Feed review queue | `840a833b` |
| 19 | Feed library | `e420c4b5` |
| 20 | Inbox triage on the Mac | `4aaccef5` |
| 21 | Train connections | `77ae56f7` |
| 29 | A record's title is its file name, so the note header renames it: `POST /projects/api/record/rename` moves the file within its folder and rewrites every `[[old]]`, `[[old|alias]]` and `[[old#Heading]]` that named it (not one written as code), refusing a taken name. Mac only; the phone's title stays read-only because its replica is keyed by path. The same change keeps each block's reader view between frames in the note editor and notifies once per click, so a click no longer parses every visible block twice. | `ebf64d4c`, `63cf19e3`, `a93a6445`, `0aac9507` |

### Open

All five are code-complete in the working tree and not committed. Each one waits for the
manual acceptance test in the running app. The agent that builds People or Finance work never
reads People or Finance records. It uses source, planted fixtures and synthetic tests.

| WP | Scope | Done when |
|---|---|---|
| 22 | Agent seat on Mac and iPhone | See [`agent.md`](agent.md#7-live-validation) |
| 23 | People writes on macOS: create, edit, delete, facts, field declaration. The phone stays read-only because entities have no phone outbox. | A manual test checks new, edit with a cleared value, add and remove fact, two-press delete, field declaration. Writes refresh the list in place. |
| 24 | Finance CSV import with mapping preview, candidate review, transfer matching, reclassification, spending review, trip and reimbursement links, balance snapshots. File input uses GPUI `prompt_for_paths`. The app sends file contents, never a path. | A manual test reviews a mapping before import and accepts, rejects and recategorizes candidates. Balances show on the page. |
| 25 | Scouting and Discover in Feed: sources, scans, saved opportunities, status, calendar promotion with provenance | `/scout` and Discover links stay in GPUI. Scan, save, dismiss, restore and promote work. |
| 26 | Inventory and wishlist create, edit with the read revision, state changes. Inventory stays the only writer. | Both views refresh without restart. A stale edit is refused and the entry reloads. State history and layout impact preview stay separate work. |
| 27 | Low-fare alerts on the iPhone: rule in `core/src/travel.rs`, unseen `sparpreis-low:` notes through the local notification host, ten-minute throttle, works from saved trips offline | A new low notifies once. A repeat check does not. It works offline. |
| 28 | Wish comparison. Built: a wish group's candidates carry numeric specs (`inventory specs`, `PUT /api/items/{id}/specs`), the group carries needs (`inventory needs`), and each candidate has dated price observations, one per source and day (`inventory price`, `POST /api/items/{id}/prices`). `GET /api/wish-comparison` reads it all. The daily job [`wish-prices`](../../capabilities/wish-prices/README.md) runs `inventory prices refresh`. It reads only geizhals product pages, never a search page, which geizhals' `robots.txt` disallows. It identifies itself as Sjel, never as a browser, and takes the page's `og:price:amount`. The first price at or below `cost_max_cents` is the alert. The wishlist tab shows, for the selected group, price against a chosen spec with the need and limit lines, a price history, and per candidate the latest price, fit to needs, and € per unit of each spec. The rules are in `core/src/inventory_compare.rs`. A wish group marked as alternatives (`inventory alternatives`, the "Pick one" button) counts only its cheapest member in every budget. | Built: a daily run adds one observation per candidate, and a second run on the same day adds none. A search URL is refused as a source. A price at the limit alerts once (`capabilities/inventory/tests/compare.rs`). Not run: the manual check in the app with three priced candidates. |

### Notes editor: live preview (open)

Clicking a block leaves its rendered text for its markdown, so the line reflows: `**bold**` takes
the width of `bold`, and a link shows its URL. Obsidian hides those markers and shows them only
where the caret is. Neither `gpui-base`'s input nor `gpui-kit`'s `Textarea` can hide a range — they
expose fold ranges for whole lines and highlight runs — and `apps/gpui` patches no crate.

Measured (release): naming a block's concealed ranges and mapping every source byte to the shown
one costs 1.5 µs a block, 80 µs a typical note and 3.0 ms for the largest of 1,230 notes
(`Inline::concealed` and `Inline::shown_offset` in `core/src/inline.rs`, `concealment_cost
--ignored`). A keystroke touches one block, so the work live preview adds is ~2 µs: the cost sits
in shaping and editor integration, not in the mapping.

Prototype shipped in `bdcd538b`: one paragraph uses route 2 behind `SJEL_GPUI_LIVE_PREVIEW=1`;
the default editor is unchanged. Implementation: `core/src/inline.rs`, `ui/src/editor.rs` and
`ui/src/editor/preview.rs` under `apps/gpui/`.

Decided 2026-11-03: prototype route 2 on one paragraph behind `SJEL_GPUI_LIVE_PREVIEW=1`, then
review before choosing a full implementation. The flag is off by default. The first paragraph's
existing textarea keeps the source markdown, keys, composition and undo history; a separate
view draws its projected text, marks, selection, preedit underline and a steady caret. The drawn
text owns the block height; the transparent input is absolutely positioned and clipped. Other
blocks keep the existing editor. Converting that paragraph into another kind ends its preview.

Prototype checks: parser tests cover nested spans, links, Unicode and source/projection offsets;
headless window tests cover unchanged wrapping on focus outside a span, click-to-source mapping,
editing inside bold with undo, typing with undo/redo, and composition commit with undo. These
checks do not establish real-device IME behavior or visual acceptance. At `bdcd538b`, the full
core and UI test suites, `cargo test -p sjel-projects`, the desktop app tests, host
`cargo check --all-targets`, Rust 2021 formatting and `bash -n apps/gpui/bench-scroll` passed.
Independent review passed for the prototype scope. Clippy completed with warnings outside the
changed implementation. The bench launcher now forwards the click mode and preview flag. Installation, click/typing measurements,
manual rename checks and an iOS build have not run. Installation was deferred while concurrent
GPUI edits were in flight; app review remains the next gate before a full-build decision.

Open before a full build: projected vertical navigation and IME candidate-window geometry
(the textarea still uses source layout), bidirectional drag selection, double/triple-click
selection, caret blinking and touch/accessibility behavior. The two layouts are not equivalent;
the prototype only prevents the hidden input's larger box from setting the displayed height.
The flag does not enable preview on every block. No full route has been selected.

The candidate routes remain:

1. **Projected text.** The textarea holds the shown text; a change is mapped back onto the markdown
   and the caret re-set. Caret, selection and hit-testing stay native. Risks: resetting the value
   per keystroke drops the input's undo history, and an IME composition's offsets move under it.
2. **Own text, invisible input.** `formatted()` draws the block, extended with a caret, a selection
   and the revealed markers; a transparent textarea stays mounted for keys, IME and undo. Risks:
   caret, selection and preedit painting are ours, and both layouts must agree on the block's
   height.

#### Resume checklist

Paused 2026-11-03 after the one-block prototype. No full-build route is authorized. Continue in
this order:

1. **Review the prototype in the app.** Read `AGENTS.md`, load the `sjel` skill, run
   `git status --short --branch` and `tools/doctor`, and preserve concurrent work on `main` in the
   primary checkout. Install only when no other session has uncommitted GPUI changes. Select a
   project-relative note path with the user; no benchmark target has been selected. Check the
   first paragraph with the flag off and on: focus outside a span keeps wrapping, bold stays
   styled, delimiters reveal only under the caret, and typing, selection, undo/redo and actual
   IME composition remain usable. Headless tests are not visual acceptance.
2. **Measure click and typing cost.** Use the same note and release build with the flag off and
   on. Record p50, p95, maximum CPU and frames over 8.3 ms in §9. No click or typing baseline
   exists yet. The benchmarks disable note saving. `NoteEditor::bench_type` targets the last
   block and `bench_click` cycles through every block, but the prototype covers only the initial
   first paragraph. Use a user-approved one-paragraph note to isolate the prototype, or add
   explicit block targeting before interpreting a multi-block run as its cost. Use
   `CARGO_TARGET_DIR="$HOME/.cargo-target"` for the app build; capability binaries under
   `./target/release` may be stale, so use `$HOME/.cargo-target/release` for any capability CLI work.
3. **Choose the full-build route with the user.** Present the app findings and measurements,
   including the source/projected geometry mismatch, before extending beyond one paragraph.
   Resolve the open input/layout risks above or choose projected text with an explicit undo/IME
   strategy. Done when the chosen route passes real app checks without unrelated spans revealing
   their source, unintended wrapping changes or lost edits/history. Do not enable it by default
   before that acceptance check.
4. **Validate rename separately.** In the app, click the title and confirm Enter moves the file
   and updates `[[links]]`; Esc and blur revert. Check a non-round-tripping note, a hub note and
   the phone's read-only title. Build the iOS target and verify on the phone; neither has run
   since the rename change. Rename spans `capabilities/projects/src/{ops,record,store}.rs`,
   `apps/gpui/core/src/{shell.rs,data/repo.rs}` and `apps/gpui/ui/src/{projects,editor}.rs`.

After the installation precondition is met, substitute the selected note path:

```sh
CARGO_TARGET_DIR="$HOME/.cargo-target" apps/gpui/install
SJEL_GPUI_LIVE_PREVIEW=0 SJEL_GPUI_BENCH_CLICK="<project>/<Note>.md" apps/gpui/bench-scroll projects
SJEL_GPUI_LIVE_PREVIEW=1 SJEL_GPUI_BENCH_CLICK="<project>/<Note>.md" apps/gpui/bench-scroll projects
SJEL_GPUI_LIVE_PREVIEW=0 SJEL_GPUI_BENCH_NOTE="<project>/<Note>.md" apps/gpui/bench-scroll projects
SJEL_GPUI_LIVE_PREVIEW=1 SJEL_GPUI_BENCH_NOTE="<project>/<Note>.md" apps/gpui/bench-scroll projects
```

Known rename gaps, not fixed by the prototype:

- Typing during an in-flight rename is lost when the note reopens. Preserve or prevent those
  edits before claiming rename is lossless under concurrent typing.
- Renaming a hub moves `P/P.md` to `P/Other.md` but leaves the folder name unchanged. Folder
  semantics need a product decision before changing that behavior.
- List rows have no rename affordance; only the open note's title supports it.
- `ops::save` compares `base` in one read and writes in another. A write between them can be
  overwritten. Make comparison and write atomic in `store::put` before claiming stale-base safety.

For the next implementation change, use the shared Cargo target and rerun:

```sh
export CARGO_TARGET_DIR="$HOME/.cargo-target"
cargo test -p sjel-projects
cargo test --manifest-path apps/gpui/core/Cargo.toml
cargo test --manifest-path apps/gpui/Cargo.toml -p sjel-gpui-ui
cargo check --manifest-path apps/gpui/Cargo.toml --all-targets
rustfmt --edition 2021 --check <changed Rust files>
```

Commit only the task's paths on `main`, state the checks that ran and did not run, and update this
section with the remaining acceptance work. No installation, benchmark, rename check or iOS
build was performed for this handoff.

## 5. Finder and right edge

- **Finder (⌘K).** One input for search or a prompt. Modes are `Auto | Ask | Search | Agent`.
  The Ask page is gone. The finder draws the conversation, and the model choice is a footer chip.
  Local results show at once. Server results come from `GET /api/sjel-status/search`
  (`capabilities/sjel-status/src/status/search.rs`), five rows per source.
- **Open: search quality.** `LIKE` folds ASCII case only ("Ä" does not find "ä"), and document
  text is not searched, only titles. An FTS5 index fixes both. Do it when a missed result costs
  something.
- **Right edge.** One frame (`ui/src/sidebar.rs`) with seats: soundscape, the phone's activity,
  and the agent run. On the phone it is a 340 pt overlay. On the Mac it docks.
- **Open: Sjel works in view.** The later adaptive UI: while the agent runs a task, the main
  column follows its steps (search, open a page, change a value). The agent's step stream is the
  input. Not designed yet.

## 6. One UI crate, every platform

1. **gpui-kit 0.8.0.** The app pins `gpui-kit = "=0.7.3"` and `gpui-pre = "=0.3.9"` in three
   manifests, and `gpui-mobile` at git rev `c24a1308e1`. Move all four pins in one commit of its
   own, with `cargo test` on core and ui and `ios/run-sim` before and after.
2. **Pages into `ui/`.** Twelve Mac pages moved to `ui/` on 2026-10-24 (`b2fb5246`, `2ce818de`), and
   eleven more modules (`documents`, `feed`, `scout`, `clipper`, `context`, `people::page`,
   `calendar::page`, `inbox::triage`, `import`, `music`, `ask`) moved on 2026-11-03 behind
   `PdfRenderer` and rich clipboard callbacks. Still in `src/`: `capabilities`, `docs` (Mermaid
   renderer), `accounts` and `settings`. A platform call goes behind one small trait in the shell
   (keychain, open URL, audio, notifications, scanner), never as a `cfg` in a page. Run `simplify`
   on each page as it moves.
3. **Phone tabs for moved pages.** The phone bar is full at five (`Today | Inbox | Calendar |
   People | Travel`, `ui/src/nav.rs`). A sixth page needs a different bar, not a sixth item.
   Next on the phone: Inventory and Wishlist, then People writes (D3 in [`PLAN.md`](../../PLAN.md)).
4. **Linux, Windows, Android.** The Mac shell checks on Linux (`4cd60961`), and CI checks every
   leg (`578e6948`). Linux credentials use the Secret Service (`37adf658`). Open: a Windows
   keychain crate, a PDF renderer off macOS, a launch on a real Linux desktop, packaging, and an
   Android host like `ios/host`. The pinned `gpui-mobile` already has an Android backend.

## 7. UI and UX roadmap

Polish does not block retirement. The write paths do. The phone priority areas are in §2. Order
inside this list is by use.

- **Documents.** Cursor paging instead of 7,154 rows on mount (needed by §2). Page thumbnails,
  text search in the rendered PDF, drag and drop from Finder into import.
- **Finance.** A six-month spending and income chart. A keyboard category picker.
- **People.** Interaction history: notes that mention the person, with dates.
- **Inventory.** State history and layout impact preview (from Work Package 26).
- **Travel, Tasks, Notes & Feed UI/UX upgrades (2026-10-28).**
  - *Shared Markdown Block Reader:* `render_markdown` and `styled_inline` in `doc_view.rs` wired across Feed reader and Travel agenda.
  - *Travel Readiness & Day Filter:* Coverage readiness strip (nights, missing booking codes), quick day pill filter (`All days | Day N`), direct/buffer transfer badges.
  - *Projects & Tasks Power-Up:* 1-click due preset chips (`Today`, `Tmrw`, `+7d`, `Clear`), subtask progress badges (`done/total`), smart task input syntax (`!1`-`!3`, `@today`, `@tomorrow`, `@YYYY-MM-DD`).
  - *Notes & Editor Outline:* Note kind filter chips (`All | Hubs ★ | Notes ≡`), 2-line snippet preview under titles, editor task completion badge, and clickable heading jump outline (`§ H1/H2...`).
  - *Feed & Inbox Triage:* Feed row evaluation score badges, inline 1-click triage buttons (`Keep | Dismiss`), formatted Inbox capture cards with status pills and tag badges (`#task`, `#idea`, `#place`, `#price`).

## 8. Rules for a session that takes a work package

1. Do not touch or stage files under `capabilities/documents/src/`. Another session owns them.
2. Run the tests before and after:
   `cargo test --manifest-path apps/gpui/core/Cargo.toml` and
   `cargo test --manifest-path apps/gpui/Cargo.toml`.
3. Install and check the desktop build with `apps/gpui/install`.
4. A shipped work package shrinks to one row in §4 with its commit.

## 9. Frame cost

`apps/gpui/bench-scroll <page>` drives the open page's own scroller (`Workspace::bench_scroll`) and
reports p50, p95 and max CPU per frame from the probe in `src/frames.rs`.
`SJEL_GPUI_BENCH_NOTE=<record path>` types into that note instead, and `SJEL_GPUI_BENCH_CLICK=<record
path>` moves the caret into another block 240 times: what a click costs the note editor. Neither
number is taken yet. A page that owns its
scroll answers the bench itself, so a virtual list is measured where it scrolls. The 120 Hz budget
is 8.3 ms a frame. Release build, 2026-11-02:

| page | p50 ms | over 8.3 ms | page | p50 ms |
|---|---|---|---|---|
| feed | 1.7–3.7 (was 71.7) | 0–3/240 (was 240/240) | self | 4.4 |
| systems | 9.5 | 240/240 | capabilities, inbox | 3.7 |
| inventory | 7.6 | 8/240 | software, settings, cv | 2.8–3.2 |
| documents | 7.5 | 3/240 | home, research, calendar, travel | 1.4–2.6 |
| packs | 5.7 | 2/238 | setup, backup, docs, media, links, import, finance, people | 0.7–2.3 |

An empty page costs about 1.6 ms, so almost all of the figure above is the page's own rows: a rich
Feed row costs about 0.2 ms to build, layout and paint.

### Done

| # | What | Result |
|---|---|---|
| E1 | Feed's Reading list draws one row per item on screen: `ListState` and `list()` in `src/feed.rs`, the order cached in `Feed::reading_order` and rebuilt only when the entries, the filter or a status move. Feed owns its scroller and every other tab in the page keeps one of its own. `j`/`k` reveals the cursor's row. | 71.7 → 1.7–3.7 ms p50 for 362 rows, of which about 15 are drawn. The spread is machine load across eight runs of the bench. Uncommitted. |

### In progress: E2, the same list element on the other long pages

| page | what it draws | state |
|---|---|---|
| Feed, Library tab | 1,074 archive rows | Done: `ListState` and a cached order; the `Show more` cap is gone. 26.2 ms before, unmeasured after. |
| Documents | 7,154 records, 100 drawn | Done: the per-frame clone of every record is gone, the filter and sort moved into `sync_order`, and only the rows in the window are built. 7.5–8.2 ms before, unmeasured after. |
| Inventory, Interior | grouped cards in one scroll container | Open. They build cards rather than uniform rows, so a conversion needs a looked-at window; not done blind. |
| Packs | a matrix with a pinned header and a horizontal scroller | Open, and last: 5.7 ms with 2 of 238 frames over budget. |
| Systems | six chart panels, not a row list | Not an E2 page: virtualization cannot help it. Its 9.5 ms is chart drawing every frame, which needs its own entry. |

### Open

| # | What | Done when |
|---|---|---|
| E2 | One helper in `ui/src/primitives.rs` for the list, its scrollbar and the height contract, so a page cannot repeat the zero-height mistake the first conversion made. | Three pages share it, and each `bench-scroll` shows no frame over 8.3 ms. |
| E3 | Filter, sort and `Board::from_records` leave `render()`. Done for Feed and Documents; open for `ui/src/projects.rs:1739`, `:2531`, `:2851` (`Board::from_records` per frame) and the count pills on Feed. 241 `.clone()` and 165 `format!` calls still sit inside render functions across 79 of them. | An unchanged page re-renders without an allocation that scales with its row count. |
| E4 | Poll hygiene. Done: the poll asks git at most every 5 minutes (`UPDATE_CHECK`), while ⌘⇧U and Settings → Updates still ask at once. Open: `Remote::settle` reports whether the value moved so the caller notifies only then. | An idle window paints nothing between polls, and one poll cycle forks no process. |
| E5 | Systems redraws six chart panels every frame. Its beats (`MACMON_BEAT`, `AGENT_BEAT`) re-render whether or not the reading moved, and the charts are rebuilt rather than reused. | A beat that reports the same reading paints nothing. |
