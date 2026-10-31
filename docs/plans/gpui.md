# GPUI: one native app, and the dashboard retires

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

1. Move the page into `ui/` behind a `PdfRenderer` trait in the shell. PDFKit implements it on
   macOS and iOS.
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

1. **Itinerary timeline.** One card per day. Each stage shows its mode, times and the connection
   time to the next stage. Trip details include linked events and nearby calendar events, each
   with its full start–end time range and any extracted programme details; nearby matches stay
   marked as unlinked until explicitly added to the itinerary. A tap copies the booking code.
2. **Map.** Category pins for stays, stations, airports and places. A tap opens a card with the
   stop's name, address, date, and "Open in Apple Maps". A "fit trip" control.
3. **Offline-ready mark.** Shown per trip when its details, its booking PDFs (kept under rule 2)
   and the map tiles for its bounds at zoom 10–15 are on the phone.
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

Retirement needs three things beyond ported pages. No link falls back to the browser. No rule
lives only in TypeScript. Nothing in the repository needs `dashboard/` to build or pass.

**Pages outside the matrix.** `/travel/connections`, `/feed/library`, `/feed/review`, `/apps`,
`/docs`, `/panel/[name]`, `/record` and `/trips` are ported. Two are not finished:

- `/feed/[id]` (1,929 lines): partial. GPUI reads the detail, edits calendar entries, handles
  Google export consent and export, and previews and approves a local cloud derivative.
- `/scout`: built as Feed's Discover tab (Work Package 25). manual acceptance test pending.

**Rules that live only in TypeScript.** `dashboard/vite/*.test.ts` tests about 40 modules in
`dashboard/src/lib`. These had no Rust owner when checked by name: `merge-candidate`,
`who-is-around` and `mail-model-proposal`. `finance-csv-mapping` and `finance-spending-review`
now have Rust versions in `core/src/finance.rs`. Move each remaining rule into a capability or
`core` before its page is called done.

**Infrastructure.**

- `dashboard/src-tauri` is an optional native shell, not a supervised service. A Tauri iOS build
  may still be installed on a phone (bundle ID `com.larsboes.sjel`). Check this on the device
  before removing the target.
- `sjel-status` serves `dashboard/dist/index.html` as its UI fallback. `dashboard_freshness`
  watches that bundle, and distribution packaging copies it. Remove the fallback and the bundle
  copy. Keep the API host.
- `/api/top-processes` is Vite dev middleware with no owner.
- 78 files in `tools/`, `capabilities/`, `libs/`, `Packs/` and `schemas/` name `dashboard/`.
  `route.rs` `matches_dashboard_nav` reads `nav.ts` and goes with it.
- The public demo (`demo/`, published at `larsboes.github.io/Sjel/`) is the dashboard. It goes
  with the dashboard. See [`restructure.md`](restructure.md) for what replaces the public page.

**Gates (2026-10-25).**

1. Delete `dashboard/` only after Work Packages 23 and 24 pass the manual acceptance test.
2. Keep `dashboard/src-tauri` until three more things are true. Foundation Models availability is
   ported to GPUI or retired. Fare alerts (Work Package 27) are verified on a device. Legacy
   RoomPlan drafts migrate on a device that holds real Tauri scans.
3. Then remove the 78 references, `matches_dashboard_nav`, ⌘D "Open in the dashboard", the
   `sjel-status` static serve, and `demo/`.

**Measure what is left.** Every dashboard handoff appends one line to
`~/.config/sjel/dashboard-fallbacks.tsv` (`Shell::dashboard_url`). Read the counts with
`cut -f2 ~/.config/sjel/dashboard-fallbacks.tsv | sort | uniq -c | sort -rn`. After a week of use
the counts are the gap list.

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
2. **Pages into `ui/`.** Twelve Mac pages moved to `ui/` on 2026-10-24 (`b2fb5246`, `2ce818de`).
   Still in `src/`: Documents (PDFKit), Feed (needs Documents), Capabilities, and the Mac wrappers
   `people_view`, `context_view` and `calendar_view`. A platform call goes behind one small trait
   in the shell (keychain, open URL, audio, notifications, scanner), never as a `cfg` in a page.
   Run `simplify` on each page as it moves.
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
