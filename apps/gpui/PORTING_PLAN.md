# GPUI Migration & UI/UX Evolution Plan

> **Strategic Mandate (2026-10-22)**: The SvelteKit + Tauri dashboard (`dashboard/`) is the secondary consumer and is retiring. The native GPUI client ([`apps/gpui`](README.md)) is the primary, universal application across macOS desktop, iOS mobile, and future Linux/Windows targets.
> All business rules, data schemas, and network adapters live in shared crates ([`apps/gpui/core`](core/README.md) and [`apps/gpui/ui`](ui/README.md)).

---

## 1. Executive Status Matrix (All 25 Dashboard Routes)

| # | Route | Page / Destination | Status | Desktop Client | iOS Mobile Client | Capability / Source |
|---|---|---|---|---|---|---|
| 1 | `/` | Home (Command Center) | **Ported** | Native GPUI (`Home` + `ContextPage`) | Native GPUI Tab (`Today`, below) | Sjel Shell / Core |
| 2 | `/context` | Today (Timeline & Focus) | **Ported** (`/` and `/context` are one page) | Native GPUI (`Home` + `ContextPage`) | Native GPUI Tab (`Today`: the day off the copy, the tasks and projects off the replica) | `capabilities/context`, `capabilities/projects` |
| 3 | `/calendar` | Calendar (Week/Month) | **Ported** | Native GPUI (`CalendarPage`) | Native GPUI Tab (the replica, writes included; the Mac's copy until the first pull) | `capabilities/calendar` |
| 4 | `/projects` | Projects, Tasks & Notes | **Ported** | Native GPUI (`Projects`) | Native GPUI Tab | `capabilities/projects` |
| 5 | `/documents` | Documents & PDF Reader | **Ported** | Native GPUI (`Documents`) | Planned Reader | `capabilities/documents` |
| 6 | `/inbox` | Capture Inbox & Triage | **Ported** | Native GPUI (`Inbox`) | Native GPUI Tab (capture only: triage stays on the Mac) | `capabilities/comms` |
| 7 | `/feed` | Comms Feed & Mail Triage | **Ported** | Native GPUI (`Feed`: Reading, Review, Library, Mail) | Planned | `capabilities/comms` |
| 8 | `/travel` | Trip Planner & Itinerary | **Partially Ported** | Native GPUI (`Travel`) | Native GPUI Tab | `capabilities/travel` |
| 9 | `/map` | Interactive Travel Map | **Partially Ported** | Native GPUI (`Travel::Map`) | Native GPUI Map | `capabilities/travel` |
| 10 | `/finance` | Financial Ledger & Subs | **Ported** | Native GPUI (`FinancePage`) | Planned | `capabilities/finance` |
| 11 | `/inventory` | Household Assets | **Ported** | Native GPUI (`InventoryPage`) | Planned | `capabilities/inventory` |
| 12 | `/wishlist` | Wishlist & Budgeting | **Ported** (Tab) | Native GPUI (`InventoryPage`) | Planned | `capabilities/inventory` |
| 13 | `/interior` | Spatial Layouts & Rooms | **Ported** (Tab) | Native GPUI (`InventoryPage`) | Needs RoomPlan | `capabilities/inventory` |
| 14 | `/people` | People & Contacts CRM | **Ported** | Native GPUI (`PeoplePage`) | Native GPUI Tab (`People` \| `Places`, off the copy) | `capabilities/people-registry`, `capabilities/entities`, `capabilities/places` |
| 15 | `/software` | App Manager & Decisions | **Ported** | Native GPUI (`Software`) | Desktop Only | `capabilities/software` |
| 16 | `/backup` | Backup Deep Inspection | **Ported** | Native GPUI (`BackupPage`) | Status Only | `capabilities/backup` |
| 17 | `/capabilities`| Capability Manager | **Ported** | Native GPUI (`Capabilities`) | Status Only | Sjel Shell / Daemon |
| 18 | `/settings` | Settings & Shortcuts | **Ported** | Native GPUI (`Settings`) | Native Tab | Local GPUI State |
| 19 | `/systems` | Systems & Host Telemetry | **Ported** | Native GPUI (`SystemsPage`) | N/A | `capabilities/sjel-status`, `macmon` |
| 20 | `/packs` | Packs & Harness Sync | **Ported** | Native GPUI (`PacksPage`) | N/A | `tools/harnesses` |
| 21 | `/self` | Self-Model & Knowledge | **Ported** | Native GPUI (`SelfPage`) | N/A | `self.json` / `graphify` |
| 22 | `/cv` | CV Builder & Applications| **Ported** | Native GPUI (`CvPage`) | N/A | `cv` CLI |
| 23 | `/media` | Media Ingest Ledger | **Ported** | Native GPUI (`MediaPage`) | N/A | `capabilities/media` |
| 24 | `/research` | Local Models & Evals | **Ported** | Native GPUI (`ResearchPage`)| N/A | `research/`, `upstreams.toml`, `systems.toml` |
| 25 | `/setup` | Node Setup & QR Pairing | **Ported** | Native GPUI (`SetupPage`) | Native Scanner | `capabilities/devices`, `capabilities/sjel-status` |

The matrix lists 25 routes. `dashboard/src/routes` holds 36 pages, and §1b names the rest.

---

## 1b. What retiring the dashboard still takes (2026-10-24)

The matrix counts pages. Retirement needs more: no link may fall back to the browser, no rule may
live only in TypeScript, and nothing in the repository may need `dashboard/` to build or pass.

**Pages the matrix does not track.** Every `sjel://` link to one of these lands on
`Target::Dashboard` (`core/src/route.rs`) and opens the browser.

| Route | Lines | In GPUI |
|---|---|---|
| `/travel/connections` | 817 | ported: Travel's connection search (Work Package 21) |
| `/feed/library` | 373 | ported: Feed's Library tab (Work Package 19) |
| `/feed/review` | 363 | ported: Feed's Review tab (Work Package 18) |
| `/apps` | 217 | in `DESTINATIONS`, no `Page` (`Software` is `/software`) |
| `/docs`, `/docs/[...slug]` | 211 | in `DESTINATIONS`, no `Page` |
| `/panel/[name]` | 111 | no page |
| `/record` | 99 | no page (a record as its own page, after the row and the inspector) |
| `/feed/[id]`, `/scout`, `/trips` | ≤ 18 each | not checked, probably redirects |

**Writes that still go to the browser.** Travel cannot create a trip, a booking or a stage (§4).
Editing a person, adding to the wishlist and
recategorizing a transaction stay on the web. Fare alerts live only in
`dashboard/src/lib/travel/fare-alerts.ts`.

**Rules that live only in TypeScript.** `dashboard/vite/*.test.ts` tests about 40 modules in
`dashboard/src/lib`. Some have no Rust owner: `finance-csv-mapping`, `finance-spending-review`,
`merge-candidate`, `who-is-around`, `mail-model-proposal`, `fare-alerts`. Checked by name only, so
some may already have a Rust twin. Each rule moves into a capability or `core` before its page
ports.

**Infrastructure.**
- `dashboard/src-tauri` holds 5,433 lines (`sync.rs`, `mac_bridge.rs`, `mirror.rs`). `core` has
  its own `sync.rs` and `mirror.rs`. Check that no device still runs the Tauri build, then delete it.
- RoomPlan part 2 (§3 item 5) ports the Swift out of `plugins/roomplan`.
- `sjel-status` serves `dashboard/dist` as the spine's shell (`dashboard/service.toml`). Decide
  what that port serves after retirement.
- `/api/top-processes` is Vite dev middleware and has no owner (Work Package 0).
- 78 files in `tools/`, `capabilities/`, `libs/`, `Packs/` and `schemas/` name `dashboard/`.
  `route.rs` `matches_dashboard_nav` reads `nav.ts`, and goes when `nav.ts` does.

**Order.** UI polish (§4) does not block retirement. The write paths do.

1. **Measure the fallbacks.** Every dashboard handoff appends one line to
   `~/.config/sjel/dashboard-fallbacks.tsv` (`Shell::dashboard_url`). After a week of use, the
   counts are the gap list, and they replace the estimates above. Read them with
   `cut -f2 ~/.config/sjel/dashboard-fallbacks.tsv | sort | uniq -c | sort -rn`.
2. **Travel authoring and connections.** The largest gap, and §4's highest priority. Authoring
   shipped on the Mac in Work Package 17, connections in Work Package 21.
3. **Feed review and library, then Inbox triage.** That finishes comms. Review shipped in Work
   Package 18, the library in Work Package 19, Inbox triage in Work Package 20.
4. **The small pages:** `/record` (the inspector exists), `/docs`, `/apps`.
5. **Delete `dashboard/`.** Then the 78 references, `matches_dashboard_nav`, ⌘D "Open in the
   dashboard", and the `sjel-status` static serve.

---

## 2. In-Progress & Next Work Packages for Agent Sessions

### ✅ Work Package 0: Systems & Host Telemetry (`/systems`)
- **Status**: Ported (2026-10-23).
- **Shipped**: `apps/gpui/core/src/systems.rs` (payloads and the pure reductions: storage ordering and
  units, the update grouping and apply summary, the trend bars, dated absences, the runtime profile
  line) and `apps/gpui/src/systems.rs` (the page). Six panels, one per owner: macmon's live sample on
  a 3 s clock with a Start button while it is stopped, the runtime profile (selection and On-the-go
  exceptions), the agent's reach (per-capability mode, pending approvals, latest calls), storage
  with the disk's hourly history, updates with one apply button per class, and the hourly host
  watch. `⌘9 › Systems` or `g x`.
- **Not here**: `/api/top-processes` is a Vite dev-server middleware and has no owner on this side,
  so the largest-RAM-users popover stays on the web page until something serves it.

### ✅ Work Package 1: Packs & Harness Deployments (`/packs`)
- **Status**: Ported (2026-10-23).
- **Shipped**: `apps/gpui/core/src/packs.rs` (data models, wire payloads, matrix rows calculation, attention detection, and RPC helpers) and `apps/gpui/src/packs_view.rs` (`PacksPage` with harness summary cards, orphan status alerts, interactive skill-by-harness matrix grid with deploy/sync cell buttons, stray inspections, and filtering). Wired into `route.rs` as `Page::Packs` (`/packs`, `g p`).

### ✅ Work Package 2: Self-Model & Knowledge Architecture (`/self`)
- **Status**: Ported (2026-10-23).
- **Shipped**: `apps/gpui/core/src/self_model.rs` (data models for `self.json` and `/api/sjel-status/self`, coupling degree calculation, isolated unit reductions, and live capability mapping) and `apps/gpui/src/self_view.rs` (`SelfPage` with stats summary, architectural unit cards, liveness dots, interactive inspector for incoming/outgoing compile-time couplings and runtime dependencies, and service start/stop/restart lifecycle controls). Wired into `route.rs` as `Page::SelfModel` (`/self`, `g l`).

### ✅ Work Package 3: CV & Career Applications Ledger (`/cv`)
- **Status**: Ported (2026-10-23).
- **Shipped**: `apps/gpui/core/src/cv.rs` (data models for `master_cv.yaml`, applications, logs, built PDFs staleness, slug generation, filtering, and shell RPC methods `cv()` and `write_cv()`) and `apps/gpui/src/cv_view.rs` (`CvPage` with master info header, profile switcher, language toggles, PDF staleness badge, one-click PDF builds, LinkedIn pull/diff/paste-sheet actions, full applications ledger table with quick status transitions, app build triggers, and live capture form). Wired into `route.rs` as `Page::Cv` (`/cv`, `g y`).

### ✅ Work Package 4: Media Ingest Ledger (`/media`)
- **Status**: Ported (2026-10-23).
- **Shipped**: `apps/gpui/core/src/media.rs` (data models for `MediaStatus`, `MediaVolume`, `MediaIngestRun`, `MediaIngestItem`, `MediaIngestDetail`, byte formatting, leaf run labels, attention sorting, and shell RPC methods `media_status()`, `media_ingests()`, `media_ingest()`, `media_view()`) and `apps/gpui/src/media_view.rs` (`MediaPage` with index totals header, mounted volume breakdown table, recent ingest runs list with status tallies, and full run inspector with disposition filter tabs and item status badges). Wired into `route.rs` as `Page::Media` (`/media`, `g w`).

### ✅ Work Package 5: Research & Local Decision Models (`/research`)
- **Status**: Ported (2026-10-23).
- **Shipped**: `apps/gpui/core/src/research.rs` (data models for `UpstreamRow`, `SystemRow`, `ProjectGroup`, `BenchmarkCase`, `BenchmarkResult`, `BenchmarkRun`, `BenchmarkSuite`, `ResearchArticle`, `ResearchView`, pure single-line TOML parser, upstream and suite grouping, and embedded fallback research view) and `apps/gpui/src/research_view.rs` (`ResearchPage` with 3 core tabs: Articles reader with full markdown document rendering, Projects register with search filter, "Runs alongside" systems table, upstreams grouped by verdict, "Add project to watch" live form backed by `watch_upstream()`, and Benchmarks suites evaluation tables with model latency/accuracy and per-case pass/fail breakdown matrix). Wired into `route.rs` as `Page::Research` (`/research`, `g z`).

### ✅ Work Package 6: Node Setup & Device Pairing (`/setup`)
- **Status**: Ported (2026-10-23). Both ends: the Mac draws the code, the iPhone reads it.
- **Shipped**: `apps/gpui/core/src/setup.rs` (the setup plans and their probes, the listener, the
  one-time challenge, the `sjel-pair/1` payload, the QR matrix, and the pure reductions: the
  verification merge, the step count, the active/revoked partition, the waiting joins, the
  countdown and the short pin) and `apps/gpui/src/setup_view.rs` (`SetupPage` with two tabs:
  **Machine Setup** — probes, plan steps, and one apply form whose credential is piped to the
  tool's stdin and then dropped, never shown again; **Devices** — the Same-Wi-Fi listener, a
  single-use code drawn as a QR `canvas` plus its copyable payload, join requests with the key code
  both screens show, the trusted keys with certificate, trust and last-seen dates, and a two-press
  revoke). Wired into `route.rs` as `Page::Setup` (`/setup`, `g 0`).
- **Phone**: `apps/gpui/ios/host/PairScanner.swift` presents `AVCaptureMetadataOutput` and hands
  the text to `sjel_ios_scanned_pairing`, which makes the same claim the `SJEL_PAIR` launch path
  makes. `UITests/ScanTap.swift` proves a finger reaches it (`ios/run-sim --tap`).
- **Not here**: revoking this device from the phone, and the phone's own copy of the device list.
- **Data Source**: `capabilities/devices`, `capabilities/sjel-status`
- **Target Files**:
  - `apps/gpui/core/src/setup.rs`
  - `apps/gpui/src/setup_view.rs`

### ✅ Work Package 7: the phone's Calendar tab, off the Mac's read-only copy
- **Status**: Native iOS tab (2026-10-23). Shipped to TestFlight as build 1363. Read-only when it
  shipped; it has written the replica since Work Package 8, so read this package for how the copy
  is fetched, not for what the tab does now.
- **Shipped**: `apps/gpui/core/src/mirror.rs` (the manifest, the digest- and byte-checked fetch that
  stages and renames into place, the rebuild request, and the day read; 8 tests against a fake Mac
  and a real SQLite copy) and `apps/gpui/ui/src/calendar.rs` (one day, seven day chips with a dot on
  every day an entry covers, entry rows at `Density::Mobile`, the copy's built-at line, and a
  Refresh that asks the host rather than reaching the network itself). `apps/gpui/ios/src/lib.rs`
  carries the third tab, opens the copy on disk at launch, and fetches a newer one when the pairing
  answers or Refresh is tapped.
- **Why the copy and not the replica**: the copy is one file the Mac builds atomically and serves
  with a digest, so two more table sets cost the Mac nothing and the phone one fetch. It is
  read-only by intent; writes stay in the replica, which covers projects.
- **Data Source**: `sjel mirror build`, `GET /api/sjel-status/mirror` (+ `/rebuild`)
- **Target Files**:
  - `apps/gpui/core/src/mirror.rs`
  - `apps/gpui/ui/src/calendar.rs`
  - `apps/gpui/ios/src/lib.rs`
- **Next**: People and Places are a page away, because `places_*` and `entities_*` are already in
  the phone's table set. The pages that write need the replica's second domain, which is the same
  shape as `core/src/sync.rs` per capability.
  *(Shipped in Work Package 9; the tables the page needed, `entities_values` and `places_people`,
  were not in the set yet and were added there.)*
### ✅ Work Package 8: the calendar joins the replica, and the phone's tab writes
- **Status**: Shipped (2026-10-23). The tab reads and writes the replica; the Mac's copy is the
  fallback until the first pull.
- **Shipped**:
  - `apps/gpui/core/src/store.rs` — the replica's second domain: a `calendar_entries` table, a
    `_replica_meta` cursor per domain, and a `domain` column on `_offline_mutations` with the
    migration that defaults an old log to `projects`.
  - `apps/gpui/core/src/sync.rs` — `push` per domain (one refused calendar write cannot hold up a
    note), `calendar_save` (a phone-minted id creates, anything else changes, and a 404 re-creates
    rather than losing the edit), `calendar_delete`, and `pull_calendar`: a change feed read from a
    cursor, tombstones applied, an id with an unsent write skipped, and the cursor moved only after
    everything in the answer has been applied.
  - `apps/gpui/core/src/data/repo.rs` — `Repo::fetch_calendar` answers from the replica with the
    full entry (id, kind, place, notes, source), plus `save_calendar_entry`,
    `delete_calendar_entry`, and `has_calendar`.
  - `apps/gpui/core/src/calendar.rs` — `Draft::entry`, so the Mac's form body and the phone's replica
    write agree about what a field means.
  - `apps/gpui/ui/src/calendar.rs` — two sources, one page: the store answers once
    `Repo::has_calendar` is true, the Mac's copy until then, and the title line says which one drew
    the day. The **New** form (title, all-day or 09:00–10:00, commitment) writes one entry on the
    open day; a row's **⋯** asks, and a second tap deletes. Both are absent over a copy.
  - `apps/gpui/ios/src/lib.rs` — hands the page the replica `Repo` when the store opens, reloads it
    after every sync, and treats **Refresh** as a sync when there is a store.
- **Why writes are thin on purpose**: a timed entry made here is 09:00–10:00 on the open day and is
  corrected on the Mac, which has the room for a time picker. Guessing hours on a phone is worse
  than two taps on a keyboard.
- **Tests**: 14 new (6 in `sync.rs`, 1 in `store.rs`, 5 in `data/repo.rs`, 1 in `calendar.rs`, 1 in
  `ui/src/calendar.rs`); `cargo test -p sjel-gpui-core -p sjel-gpui-ui` is 248 + 30, all green.
- **Target Files**:
  - `apps/gpui/core/src/{store,sync,calendar}.rs`
  - `apps/gpui/core/src/data/repo.rs`
  - `apps/gpui/ui/src/calendar.rs`
  - `apps/gpui/ios/src/lib.rs`
- **Next**: the same shape per capability. Each remaining table set needs a feed and a table before
  its page can leave the copy, and the copy's table set shrinks as they move.

### ✅ Work Package 9: People and Places off the copy
- **Status**: Shipped (2026-10-23). One new tab, two registers, read-only over the Mac's copy.
- **Shipped**:
  - `tools/sjel-cli/src/mirror.rs` — the phone set gains `entities_values` (612 rows: phones,
    emails, birthday, last_contact, company, relation) and `places_people` (the operator's decision
    for a person's name), because a copy without them drew 294 names and no contact card.
  - `apps/gpui/core/src/people.rs` — `Person::from_record` now accepts the keys the registry
    actually writes. `capabilities/entities/src/google.rs` writes `emails` and `phones` (plural,
    array of strings) while the parser looked for `email` and `phone`, so **every** contact card
    was missing its email and phone, on the Mac too (`apps/gpui/src/people_view.rs`). Fixed in the
    shared core, so both clients gained the rows.
  - `apps/gpui/core/src/places.rs` (new) — `Place`, `PersonLink`, `places_of`, `people_at`,
    `filter_places`, and the `name_key` join (re-used from `libs/links`, not copied). A dismissed
    link is not a link.
  - `apps/gpui/core/src/mirror.rs` — `Mirror::people` answers all three tables in one read
    (`PeopleCopy`), a left join so a person with no values is still a row, values parsed out of the
    JSON text the registry stores, and empty lists rather than an error on a copy that predates them.
  - `apps/gpui/ui/src/people.rs` (new) — one tab with a `People` | `Places` switch, one search field
    for both sides, initials rows at `Density::Mobile`, a contact card (email, phone, custom fields,
    the linked places) and a place card (kind, where, coordinates, the names linked to it), with the
    source line naming the copy and a Refresh that asks the host for a newer one.
  - `apps/gpui/ios/src/lib.rs` — `Tab::People`, `SJEL_TAB=people`, and the same copy the Calendar
    tab reads: one fetch feeds both. `SJEL_REGISTER=places` and `SJEL_PERSON=Ada` open the other
    register and a card at launch, for the screenshots `run-sim` takes.
- **Verified on a simulator** (`ios/run-sim`, seeded with the built copy): the People list (294
  names, 11 with a company line), a person's card (both phone numbers, the birthday as a custom
  row, "no places linked"), the Places list grouped by kind (253 rows) and a place's card (kind,
  `Amsterdam, NL`, coordinates). Three defects found and fixed there: the card's back link sat
  below the content where a long card would push it off screen (it is now at the top), the title
  said "Person" where the card already carried the name (it now stays `People`), and the
  `COORDINATES` label wrapped inside its column (`COORDS`, 92 px).
- **Why read-only**: `places_person_places` holds the operator's review and the registry's writes
  are `/api/people/decide`. A phone that cannot reach the Mac would offer controls that have to be
  refused, so it offers none.
- **Tests**: 7 new in core (1 `people`, 4 `places`, 2 `mirror`), 4 in `ui/src/people.rs`.
- **Target Files**:
  - `apps/gpui/core/src/{people,places,mirror}.rs`
  - `apps/gpui/ui/src/people.rs`
  - `apps/gpui/ios/src/lib.rs`
  - `tools/sjel-cli/src/mirror.rs`
- **Next**: the same shape per capability. Each remaining table set needs a feed and a table before
  its page can leave the copy, and the copy's table set shrinks as they move. The contact registry's
  writes (a note ref, a value, a link decision) are the replica's third domain when the tab stops
  being read-only.

### ✅ Work Package 10: Today lands, and the bar changes shape
- **Status**: Shipped (2026-10-24). `Today` is the page the phone opens on.
- **Why now**: `mobile_tab_bar` (`ui/src/nav.rs`) draws one row with `justify_around` and a 64 px
  floor per item, so five items need 320 px of the 375 pt of the narrowest current iPhone and a
  sixth needs 384. The bar was full, and `Today` plus `Inbox` could not both be added to it.
- **Decided 2026-10-24**: `Today` becomes the landing tab and `Projects` folds into it; the bar is
  `Today | Calendar | People | Travel`, `Settings` sits behind Today's gear with a `← Today` row of
  its own, and `Inbox` takes the fifth slot when it ships. `Travel` kept its slot because it is the
  one page with a full map and a phone is the device carried on the trip.
- **Shipped**:
  - `core/src/today.rs` (new) — the reduction, with no GPUI in it: the day's entries with the clock's
    marks (`Now`, and one `Next`; an all-day entry is never next because it is already under way), the
    open tasks ordered by the operator's priority then due date and capped at five with a count for
    the rest, and the projects those tasks are filed under with their progress. 12 tests.
  - `ui/src/today.rs` (new) — the page: the headline and one summary line, one line naming the source
    and its age, one line for what is queued and whether this launch is connected, then THE DAY, UP
    NEXT (a 44 pt checkbox per task, `Overdue` as a word rather than a colour) and PROJECTS with a
    progress bar and a row that opens the project. Ticking a task writes the replica optimistically
    and puts the row back if the write fails.
  - `apps/gpui/ios/src/lib.rs` — `Tab::Today` as the default, the four-item bar, `Tab::Projects` and
    `Tab::Settings` reached from Today, `SJEL_TAB=projects` kept, and `SJEL_DAY` freezing the landing
    page's day the way it freezes the calendar's.
  - `apps/gpui/ios/host/UITests/TodaySwipe.swift` (new) — drags the page up and attaches the frames:
    a screenshot cannot show that a finger scrolls a GPUI surface, and PROJECTS is below the fold.
- **A bug in shared core, found by the page**: `ProjectRecord` (`core/src/shell.rs`) dropped the
  capability's own `project` field, so `Board` could not tell which project a record was filed under
  and the first folder of the path is not the answer — the vault files `Archive/Soma/Tasks/Reopen.md`
  under `Soma`. 22 of the copy's 62 task rows would have been filed wrong, on the phone and in the
  sync's pull. The field travels now, the replica queries read the column, and `filed_under`
  (`core/src/sync.rs`) prefers it over the path.
- **Verified on a simulator** (`ios/run-sim`, replica seeded from the copy): the landing page on
  2026-10-24, one day earlier with `SJEL_DAY=2026-10-20` (one entry, `all day`), and after a drag the
  PROJECTS list, whose numbers were checked against the replica row by row — Soma 10 of 21,
  Washing Machine 1 of 8, Credential Stack 2 of 7, Hearth 4 of 8, Home-Lab 0 of 4, Life-Plan 4 of 8,
  in open-task order. Settings and Projects were screenshotted on their own tabs, and the Calendar
  tab still draws the copy it drew before.
- **Tests**: 12 new in core `today`, 1 in `sync`; 3 in `ui/src/today.rs`. core 268, ui 37, app 81.
- **Target Files**:
  - `apps/gpui/core/src/{today,shell,store,sync}.rs`
  - `apps/gpui/ui/src/today.rs`
  - `apps/gpui/ios/src/lib.rs`
  - `apps/gpui/ios/host/UITests/TodaySwipe.swift`
- **Next**: `Inbox` — the fifth slot, and the second write path — which cannot queue offline before
  #244's blocked-write recovery: a capture that cannot reach the Mac must keep its text, say why, and
  retry when discovery finds the host again.

### ✅ Work Package 11: the queue goes around a refused record (#244)
- **Status**: Shipped (2026-10-24). #244's acceptance checks are met.
- **Why**: `push` stopped at the first write the Mac refused, so one duplicate note title or one
  archived project held every later write in the queue, and no view named the record or the reason.
  A phone shown only a queue count that will not drain has nothing to act on.
- **Shipped**:
  - `core/src/sync.rs` — a refusal names its record ([`Refusal`]: path and the Mac's words), a
    refusal blocks that record and no other, and the queue keeps draining behind it. A queued row
    this build cannot parse is stepped over as a fault here rather than reported as the Mac's
    opinion. `Report::refused` carries them and `summary` says how many.
  - `core/src/store.rs` — `_sync_refusals`, one row per record, written when the Mac refuses and
    cleared when that record goes through, so the band outlives the launch that heard it and
    disappears when the problem does. A refusal is never a lost write: the record and its queued
    mutation stay exactly where they were.
  - `core/src/data/board.rs` — `Blocked { path, title, kind, reason }`, joined from the refusals and
    the replica's rows so a page can name the note rather than a path.
  - `ui/src/projects.rs` — the band above the project list: the count, each record with its kind,
    the Mac's reason in the Mac's words, "Saved here and still queued. Nothing was lost or renamed.",
    `Open` per record and one `Try again`. Nothing renames, deletes or edits a record to make a
    refusal go away: a duplicate title and an archived project are the operator's decision.
  - `apps/gpui/ios/src/lib.rs` — `ProjectsEvent::Retry` runs the sync the writes were queued for; a
    phone with no Mac says so instead of pressing a dead button.
- **Verified on a simulator**: two refusals seeded into the replica — one archived project, one
  duplicate title, the second with no record behind it — drew as two rows, the first with the note's
  own title, the second falling back to its path, and the header counting two.
- **Tests**: 3 new in `sync` (a duplicate title keeps its text, queued count and reason; an archived
  project is refused while the next write still goes; a refusal is cleared when the write lands) and
  1 in `ui/src/projects.rs` (the page holds what the sync reported and drops it when cleared).
  core 271, ui 38, app 81.
- **Target Files**:
  - `apps/gpui/core/src/{sync,store}.rs`
  - `apps/gpui/core/src/data/board.rs`
  - `apps/gpui/ui/src/projects.rs`
  - `apps/gpui/ios/src/lib.rs`
- **Next**: `Inbox` — the fifth slot and the second write path — which can now queue offline.

### ✅ Work Package 12: Inbox, the fifth slot
- **Status**: Shipped (2026-10-24). The bar is five items and `Today | Inbox | Calendar | People |
  Travel` is the shape `mobile_tab_bar` holds at its 64 px floor.
- **Why**: a phone is a capture device first (`research/offline-on-the-phone.md`), and it had no
  field for the thought in hand. It also had no second write path: only the calendar could write, so
  the outbox had one consumer.
- **Shipped**:
  - `core/src/capture.rs` — `Draft`, one typed line read the way the capability reads it: `#buy` in
    the text sets the tag (the chip wins over the hashtag, because the last thing touched is the one
    meant), a lone link becomes the URL rather than the text, an unknown `#word` stays in the text,
    and the preview says where it will land. `route_for`/`suggested_tag` are the one rule the Mac's
    triage and the phone's preview share, lifted out of `Capture` so both call the same code.
  - `core/src/store.rs` — the `captures` table and `DOMAIN_CAPTURES`: a capture is written here
    first, its own id is `capture:local:<n>`, and `rekey_capture` keeps the Mac's id once it lands.
  - `core/src/sync.rs` — a third push, `capture_new`: one `POST /comms/captures` carrying text, url
    and tag, with the capability deciding the id, the tag rule and where the capture goes. Nothing
    the phone minted is trusted, and a capture is never sent twice: it has no update path, because
    nothing on the phone edits one.
  - `ui/src/inbox.rs` (new) — the field, the four chips, the preview, `Keep`, and what was kept:
    newest first, with `#tag`, the route ("To wishlist"), and the state — `On the Mac`, `Waiting for
    the Mac`, or the Mac's refusal on the row it belongs to rather than in the projects band, which
    names records and would have shown a capture's local id.
  - `apps/gpui/ios/src/lib.rs` — `Tab::Inbox` in the fifth slot, the store handed to the page when it
    opens, a reload after every sync, and `InboxEvent::Wrote` starting one.
  - `apps/gpui/ios/host/UITests/InboxCapture.swift` (new) — taps `#buy` and `Keep`: a chip that takes
    the tag and a card that says what is missing when the field is empty. Typing is deliberately not
    driven from XCUITest, which waits for a keyboard the simulator may never raise; a check that
    hangs is worse than one that covers less, and the keystroke path is covered by
    `core::capture`'s and the store's tests.
- **Not here**: triage. Routing a capture into a wishlist item, a feed source or a place is the
  Mac's; the phone says where it will go and stops. Reading the Mac's own captures back is also not
  here — the list is what this phone captured, which is the honest thing to draw without a pull.
- **Verified on a simulator**: the empty card, a chip taken (`#buy` in accent), `Keep` with an empty
  field answering "Type what you want to keep, or paste a link.", and two seeded captures drawn with
  their tags, routes and states — `Linen shirt 49,90 €` `#buy` → To wishlist, waiting; `youtube.com`
  `#look` → To feed, on the Mac — plus, with a seeded refusal, the Mac's reason wrapped inside the
  row instead of running off the card.
- **Tests**: 2 new in `capture` (a typed line becomes a draft; an empty line is not a capture), 2 in
  `sync` (a capture typed offline reaches the Mac and keeps its id; a refused capture is kept and
  named), 1 in `store` (queued once each, newest first, landed after a rekey) and 3 in
  `ui/src/inbox.rs` (no store is empty and loaded; the count comes from the rows; the preview answers
  the chip). core 276, ui 41, app 81.
- **Target Files**:
  - `apps/gpui/core/src/{capture,store,sync}.rs`
  - `apps/gpui/ui/src/inbox.rs`
  - `apps/gpui/ios/src/lib.rs`
  - `apps/gpui/ios/host/UITests/InboxCapture.swift`
- **Next**: reading the Mac's captures back (shipped in Work Package 13), then the soundscape player,
  notifications and RoomPlan, in that order.

### ✅ Work Package 13: the Mac's captures join the phone's
- **Status**: Shipped (2026-10-24). The tab is the inbox, not this phone's half of it.
- **Why**: the capture pull was the other half of WP12. A list that hides what was captured on the Mac
  is a list of what this phone did, and a person looking for a thought they had at their desk would
  not find it.
- **Shipped**:
  - `core/src/store.rs` — the `captures` table gains the Mac's columns (`status`, `routed_to`,
    `client`), with `add_missing_capture_columns` for a device that ran the capture-only build;
    `upsert_pulled_capture` matches on the Mac's id, so a capture typed here and then landed is one
    row rather than two — matched on `remote_id` and not on the primary key, because the row keeps the
    id it was typed under. `settle_capture` takes the whole answer a create returns (id, tag, status,
    route), which is what stops the next pull reading that capture as news. `rekey_capture` now moves
    the queued write with the id, the way a calendar entry's does; without it a retry would create the
    capture twice.
  - `core/src/sync.rs` — `pull_captures`: one `GET /comms/captures`, no cursor, because the capability
    has no change feed for captures and there are hundreds at most. A capture the Mac does not have is
    left alone rather than deleted: this page is where the person's own writing lives. The count is
    what changed, not how many the Mac holds, so a quiet sync stays quiet. A capability that is down
    is not an error for the rest of the sync.
  - `ui/src/inbox.rs` — the state line in the Mac's own terms: `Waiting for the Mac`, `On the Mac`,
    `Done on the Mac`, `Dismissed on the Mac`. Saying "on the Mac" for all three would show a finished
    thought as if it were still in the inbox.
- **Verified on a simulator**: the migration added the three columns to a table written by the
  previous build, and a list mixing this phone's captures with the Mac's drew `Linen shirt 49,90 €`
  `#buy` → To wishlist, waiting · `youtube.com` `#look` → To feed, on the Mac · `Sort the tax
  paperwork` → Done on the Mac · `arxiv.org` `#learn` → To feed, on the Mac.
- **Tests**: 1 new in `sync` (the Mac's captures join this phone's, nothing is doubled, a second sync
  finds nothing new) and 1 in `ui/src/inbox.rs` (the state line follows the Mac's triage). core 277,
  ui 42, app 81.
- **Target Files**:
  - `apps/gpui/core/src/{store,sync}.rs`
  - `apps/gpui/ui/src/inbox.rs`
- **Next**: the soundscape player (an output device in `ios/host`, the panel over `core/src/soundscape`,
  and the two settings that do not persist today), then local notifications, then RoomPlan.

### ✅ Work Package 14: sound on the phone
- **Status**: Shipped (2026-10-24). The drawer opens from Today, and the device is the host's.
- **Why**: `core/src/soundscape` was already platform-free for exactly this, and the phone had no
  player: the Mac's `apps/gpui/src/audio.rs` owns a CPAL stream, which is the Mac's device and not
  the phone's.
- **Decided 2026-10-24**: the device lives in Swift, not in Rust. `AVAudioEngine` and the audio
  session are Apple's own and are what `host/` already knows how to set up; CPAL on iOS would have
  meant getting `AVAudioSession` right from Rust anyway. The contract between the halves is one call:
  the node asks for N frames into two buffers and Rust renders exactly N.
- **Shipped**:
  - `ios/host/Audio.swift` (new) — the session (`.playback` with `.mixWithOthers`: a focus soundscape is
    played *with* whatever else the phone is doing, not instead of it), an `AVAudioSourceNode` at
    48 kHz stereo, and `sjel_ios_audio_start`, which answers `ok:<device>:<rate>` or `err:<why>`.
  - `ios/src/audio.rs` (new) — the drawer's two halves ([`Control`], [`Visual`]) from the first frame,
    the [`Engine`] born on the first render because it needs the device's rate, and
    `sjel_ios_soundscape_render`, which renders one interleaved block and splits it into the node's two
    buffers. `SJEL_GPUI_NO_AUDIO=1` is honoured, the way the Mac's own switch is.
  - `ui/src/soundscape.rs` (new) — the drawer: transport with the state in words (`lofi · 9 voices ·
    playing`), the six presets, volume, a live meter from the analyser's bands, and the mixer as five
    layers. A value is a row of ten cells, not a track to measure: a thumb on a phone wants an exact
    tap. The meter asks for one frame in five, because the analyser's own release is a tenth of a
    second and a phone pays for every frame.
  - `ui/src/theme.rs` — `SoundscapePrefs` beside `appearance.json`: open state, mixer disclosure, preset
    and volume. `playing` is deliberately not kept — a phone that starts making sound by itself is a
    different thing from one that remembers its settings.
  - `apps/gpui/ios/src/lib.rs` — the drawer over the page (right edge, not a sixth tab), opened by the
    note in Today's header or by `SJEL_SOUNDSCAPE=1`, with the device taken when it opens and paused
    when it closes.
  - `apps/gpui/ios/host/UITests/SoundscapeTap.swift` (new) — play, play again, and the mixer.
- **Verified on a simulator**: the drawer with the session open (`Speaker · 48000 Hz`); playing at
  `lofi · 13 voices · playing` with the meter drawn from the analyser's bands, and at 9 voices in a
  second run; the two-way transport (playing then paused); the mixer opening on five layers; and the
  preset, volume and both disclosures restored on the next launch.
- **Not verified, and not claimed**: sound from a real iPhone's speaker — there is no device here — and
  the simulator's audio route is not stable across repeated XCUITest launches: one run showed
  `playing · 0 voices` with a flat meter, which is the drawer telling the truth about a device that
  stopped pulling frames rather than a panel that lied.
- **Tests**: 2 in `ui/src/soundscape.rs` (the drawer carries its settings into the conductor and
  remembers the next change; a preset key is a key, not a label), 1 in `ui/src/theme.rs` (the settings
  keep their shape, and a damaged file is not a half-applied panel) and 1 in `ios/src/audio.rs` (the
  drawer's halves exist before any device does). core 277, ui 45, ios 1, app 81.
- **Target Files**:
  - `apps/gpui/ios/src/audio.rs`, `apps/gpui/ios/host/Audio.swift`
  - `apps/gpui/ui/src/soundscape.rs`, `apps/gpui/ui/src/theme.rs`
  - `apps/gpui/ios/src/lib.rs`, `apps/gpui/ui/src/today.rs`
- **Next**: local notifications (shipped in WP15), then RoomPlan, then the right sidebar that the
  drawer's edge is waiting for.

### ✅ Work Package 15: the phone's own lock screen
- **Status**: Shipped (2026-10-24). Local only, and the number on the card is the system's.
- **Why**: a phone is the device you carry when an entry starts, and the replica already holds what
  would earn a reminder — the calendar's next weeks and the open tasks. Nothing here needs a server:
  there is no `registerForRemoteNotifications`, no device token and no push service in the plan.
- **Decided 2026-10-24 (operator)**: **what fires** is calendar entries plus tasks with a due date.
  Fare alerts are deferred — they exist only as dashboard TypeScript over `sparpreis-watch`
  (`dashboard/src/lib/travel/fare-alerts.ts`) and the phone has no read path for them, so including
  them here would have doubled the package. **Where the switch lives** is the Settings page, not a
  drawer off Today: a permission is a decision, and the soundscape drawer took the right edge because
  that is something you play.
- **Shipped**:
  - `core/src/notify.rs` (new) — the platform-free rule. A timed entry fires its lead (`At start`,
    `10 min`, `1 hour`) before it starts; an all-day entry at `08:00` on the day it covers, because
    subtracting ten minutes from "all day" means nothing; a task at `09:00` on its due date, for the
    same reason. `possible` blocks nothing, so it earns nothing — a reminder a person learns to swipe
    away is worse than none. Stable ids (`cal:<entry id>`, `task:<path>`) mean a re-plan replaces a
    request instead of scheduling a second one beside it, and `reconcile` hands back only what left
    the list. The cap is 60 against iOS's 64, and `Plan::dropped` says how many it left out rather
    than letting a shortened list pass for the whole of it.
  - `ios/src/notify.rs` (new) — the bridge: the whole list in, `State` out, and `SJEL_GPUI_NO_NOTIFICATIONS=1`
    honoured the way `SJEL_GPUI_NO_AUDIO` is. `apply` compares against the last list it sent, so a
    rebuild that changed nothing does not ask the notification centre to replace what it already
    holds. `ready()` is called once the view exists, because a tap launches the app and the response
    arrives while the window is still being built.
  - `ios/host/Notifications.swift` (new) — `UNUserNotificationCenter`: the prompt (asked when the
    switch is turned on, never at launch), `UNCalendarNotificationTrigger` from the epoch seconds Rust
    sends (a wall-clock moment, not a countdown, so a request is still right after a day asleep), the
    delegate that shows a banner while the app is open, and a tap's link. `report()` answers with
    `getPendingNotificationRequests`' own count and `nextTriggerDate()`, which is why the line on the
    card cannot claim more than the system holds.
  - `ui/src/theme.rs` — `NotifyPrefs` beside `appearance.json` and `soundscape.json`: the switch and
    the lead. What is *scheduled* is never read from the file; a lead key this build does not know
    falls back to the default instead of a wrong number of minutes.
  - `ios/src/lib.rs` — the reminder card at the top of Settings (a switch, the system's own answer,
    the three lead chips, the way back from a refusal through iOS Settings, and the cap's own
    admission), rebuilt when the replica opens, after a write, after a pull and when the scene becomes
    active (`sjel_ios_became_active`, from `App.swift`) — a phone in a pocket moves through days
    without the app running. A tap opens the day it announced on the Calendar tab.
  - `ios/host/UITests/NotificationsTap.swift` (new) — the switch, the system prompt answered, and off
    again.
- **Verified on a simulator**: the prompt answered from a tap and the card reading
  `Allowed · Thu 31 Dec 09:00 · Decide master program` — a task out of this phone's replica, planned
  by the rule, accepted by the notification centre and read back from `nextTriggerDate()` — then
  `Off · nothing is announced` after the second tap.
- **Not verified, and not claimed**: the banner being delivered, and a tap on it opening the day.
  Both need a reminder whose minute is about to happen, and this simulator's replica holds no
  calendar entry it may write (its calendar change feed has never been read: "The calendar was not
  read"), so nothing can be made to land inside the next minute. The trigger the system accepted does
  resolve to the moment the card shows, which is what `nextTriggerDate()` proves; delivery itself is
  Apple's half. Also not claimed: a real iPhone, and a reminder that fires while the phone is off.
- **Tests**: 5 in `core/src/notify.rs` (the lead off a timed start and the all-day hour; what earns
  nothing — `possible`, past, done, undated; the cap keeping the soonest and admitting the rest; a
  rebuild cancelling only what left; the summary and a link's day), 1 in `ui/src/theme.rs` (the
  settings keep their shape, and an unknown lead is not a wrong number of minutes) and 1 in
  `ios/src/notify.rs` (the host's answer read as the system gave it). core 282, ui 46, ios 2, app 81.
- **Target Files**:
  - `apps/gpui/core/src/notify.rs`, `apps/gpui/ios/src/notify.rs`
  - `apps/gpui/ios/host/Notifications.swift`, `apps/gpui/ios/host/App.swift`
  - `apps/gpui/ui/src/theme.rs`, `apps/gpui/ios/src/lib.rs`
  - `apps/gpui/ios/host/UITests/NotificationsTap.swift`
- **Next**: RoomPlan (LiDAR room capture into the interior layouts), which needs a LiDAR phone to
  verify, and which the operator deferred in favour of Work Package 16 below.

### ✅ Work Package 16: one right edge, with seats
- **Status**: Shipped (2026-10-24). The soundscape is one seat of the edge; the phone's own activity is
  the other.
- **Why**: §3 item 2 asks for a panel beside the page, and notes that the soundscape drawer already
  slides in from that same edge. Two panels cannot own one edge, and the plan's own open question —
  "how the two share it is decided when this is built" — had no answer that survived a person using
  both. The edge is now one object with seats, so the next thing that wants it (the agent's step
  stream) is a seat and not a third panel.
- **Decided 2026-10-24 (operator)**: the frame is shared and the seat is chosen from the panel's own
  header; the seats are the soundscape and the phone's activity. A phone has no width to dock beside
  the page, so the frame is an overlay at 340 pt from the right edge with the page dimmed beside it,
  and the same frame docks when a Mac has the room.
- **Shipped**:
  - `ui/src/sidebar.rs` (new) — `Seat` (a key, a chip's word, a title), `Sidebar` (which seat, whether
    it is showing, every change written to `sidebar.json`), and the frame: title, seat chips, close,
    and the body a host hands in. `sidebar_scrim` is the overlay form — a panel at the right edge and
    a dimmed page beside it — and it is `.occlude()`d, because a tap that closed the edge used to reach
    the page underneath it.
  - `ui/src/theme.rs` — `SidebarPrefs` in its own file; `SoundscapePrefs` loses `open`, because the
    edge is not the soundscape's. A `soundscape.json` written before this still reads: serde drops the
    key it no longer knows.
  - `ui/src/soundscape.rs` — the drawer is content now: its own header, its close button and its
    `SoundscapeEvent::Close` are gone, because the frame draws them once for every seat.
  - `ui/src/today.rs` — a second header button (≡) for the activity seat, beside the note for the
    sound seat.
  - `ios/src/lib.rs` — the edge in place of the drawer's overlay; opening takes the audio device only
    when the seat is the soundscape, switching seats keeps the sound running, and closing pauses it.
    The activity seat draws what the phone already knows — the Mac's answer, the last sync, the queue,
    and the reminders the notification centre holds — and is where the agent's steps will stream.
    `SJEL_SIDEBAR=soundscape|activity|off` opens or closes the edge for a run; `SJEL_SOUNDSCAPE` keeps
    its old meaning.
  - `ios/host/UITests/SidebarTap.swift` (new) — the ≡ button, the Sound chip, the ✕. `ScanTap.swift`
    was repaired in the same pass: it tapped a tab that has not existed since the bar became five
    tabs, and its Scan coordinate was measured before the Settings page grew.
- **Verified on a simulator**: the edge closed; ≡ opening it on `ACTIVITY` with `Sound`/`Activity`
  chips and `Reminders are off`; the Sound chip moving it to `SOUNDSCAPE` with the transport, presets
  and volume; and ✕ putting it away onto Today. `ios/run-sim --tap` is green on all seven checks.
- **A defect the tap test found**: closing the edge also opened Settings. The ✕ sits exactly where
  Today's gear does, the overlay was already gone when the finger came up, and the up reached the page.
  The scrim is now `.occlude()`d. Two real bugs in one package — this one and the drawer's remembered
  open state masking `ScanTap` — is the argument for the tap checks existing.
- **Tests**: 2 in `ui/src/sidebar.rs` (a seat is a key and an unknown one falls back; opening names a
  seat, the chips keep the edge open and closing keeps the seat) and 1 in `ui/src/theme.rs` (the edge's
  settings keep their shape; the older `soundscape.json` still reads). core 282, ui 49, ios 2, app 81.
- **Not claimed**: the docked form on a Mac, which has no host wiring yet — the frame is written for it
  and nothing on the Mac draws it. And the agent's step stream in the activity seat: the seat is real
  and its content is what the phone knows today, not a live run.
- **Target Files**:
  - `apps/gpui/ui/src/sidebar.rs`, `apps/gpui/ui/src/soundscape.rs`, `apps/gpui/ui/src/theme.rs`
  - `apps/gpui/ui/src/today.rs`, `apps/gpui/ios/src/lib.rs`
  - `apps/gpui/ios/host/UITests/SidebarTap.swift`, `apps/gpui/ios/host/UITests/ScanTap.swift`
- **Next**: the docked form on the Mac, where the width exists — the frame is shared and the Mac's
  finder overlay (⌘K) is the view that docks into it — then the agent's steps as the activity seat's
  stream. RoomPlan moved to §3 item 5, where the operator's own iPhone 14 Pro made it a package with
  a verifier.

### ✅ Work Package 17: trips are written on the Mac (§1b step 2, first half)
- **Status**: Shipped (2026-10-24). Mac only. Travel no longer sends anyone to the dashboard to
  create a trip, add a booking or change a stage.
- **Why no capability change**: `capabilities/trips` already had every route. Create, update and
  delete plans, add/edit/delete items, and `expected_updated_at` against a lost stage update were
  all there. The page was read-only because nobody had built the forms.
- **Shipped**:
  - `core/src/travel/edit.rs` (new): the three drafts and their request bodies.
    - `TripDraft` checks a date's shape, which the capability does not (it compares strings), so
      `1.11.2026` is named in the form rather than stored.
    - `BookingDraft` sends the two keys the capability requires (`provider`, `order_ref`) and
      reuses `capture::cents_from` for the amount. An empty optional field is `null` on edit,
      which removes the key. `external_id` is `booking:<provider>:<ref>`, so entering one booking
      twice updates one row.
    - `StageEdit` applies to the stages exactly as the capability returned them, so travelers,
      the chosen connection and the branch note survive an edit the form knows nothing about.
    - `typed_place` gives the id the dashboard's place field gives (`PlaceField.svelte`), so a
      city typed in either client is one place.
  - `core/src/shell.rs`: `trip_create`, `trip_booking_add`, `trip_booking_edit` (fields, then the
    day, which only the item route moves), `trip_item_delete`, and `trip_stages`. `trip_stages`
    reads the plan, edits, and writes back with the revision it read. A 409 says the trip changed
    on the Mac. Every write then asks calendar to sync the trip, as `syncTripToCalendar` did. That
    rule lived only in the dashboard's TypeScript.
  - `ui/src/travel/forms.rs` (new): one form card for all three. It opens where its button is
    (New trip on the library, Add stage and Edit on the itinerary, Add booking and Edit on saved
    items). Remove asks once, then removes.
  - `core/src/travel.rs`: `Stage` reads `id` and `transport_modes`, with defaults so a snapshot
    saved before this still opens. `plan_path` encodes every segment.
  - `core/src/travel/details.rs`: a booking row shows provider, reference and free-cancellation date.
- **Not here**:
  - The phone. It reads trips from a copy and has no outbox for them, so it shows no controls.
  - Editing the trip's own title, dates and destinations.
  - The itinerary timeline, the PNR copy button and fare alerts (§4).
  - `/travel/connections`, the second half of step 2.
- **Verified**: 5 new tests in `travel::edit`. `cargo test -p sjel-gpui-core -p sjel-gpui-ui` is
  289 + 50, all green, and `cargo clippy --workspace --all-targets` is clean on these files. Not
  yet clicked through in the running app.
- **Target Files**:
  - `apps/gpui/core/src/travel/{edit,details}.rs`, `apps/gpui/core/src/{travel,shell}.rs`
  - `apps/gpui/ui/src/travel/{forms,pages}.rs`, `apps/gpui/ui/src/travel.rs`

### ✅ Work Package 18: the feed's review queue (§1b step 3, first of three)
- **Status**: Shipped (2026-10-24). Mac only.
- **Why no capability change**: comms already served `GET /comms/feed/quality` and
  `POST /comms/feed/quality/refresh` (`capabilities/comms/src/server/main.rs`).
- **Shipped**:
  - `core/src/shell.rs`: `FeedQualityFlag`, `FeedQualityRefresh`, `feed_quality()` and
    `refresh_feed_quality()`.
  - `core/src/route.rs`: `FeedTab` is the Feed page's tab, and `sjel://feed/review` parses to
    `Target::Feed(FeedTab::Review)` instead of the dashboard.
  - `src/feed.rs`: a Review tab between Reading and Mail. One card per entry with its signals,
    reason and evidence. Recompute asks comms to derive the flags again and says what it checked.
    The title opens the entry in the reader; Original opens the source.
- **Found**: 402 of the 412 stored flags are `retention` above the 90% ceiling, all at
  `raw_chars=20000`. The ceiling fires on any source whose raw text is already clean, so the queue
  is mostly noise. That is a rule in `capabilities/comms/src/quality.rs`, not in this page.
- **Verified**: 1 new test in `feed::tests` and 1 new assertion in `route::tests`. Run against the
  live comms queue through `SJEL_GPUI_OPEN=sjel://feed/review`: 405 entries draw with their signals.
  Recompute not clicked, because it rewrites the stored flags.
- **Target Files**: `apps/gpui/core/src/{shell,route}.rs`, `apps/gpui/src/{feed,workspace}.rs`

### ✅ Work Package 19: the feed's library (§1b step 3, second of three)
- **Status**: Shipped (2026-10-24). Mac only.
- **Shipped**:
  - `src/feed.rs`: a Library tab after Review. It reads the whole archive (`GET /comms/feed`,
    3650 days, dismissed included) as one table: kind, title, author, lens, fit, status, day.
    Status pills, kind buttons with counts, and a search over title, author and link filter it.
    Rows draw 200 at a time behind Show more, as in `documents.rs`. A new row has Keep and
    Dismiss, any other row Restore. A row opens the reader, and Back returns to the library.
  - `core/src/shell.rs`: `FeedSurface::Library`, so the ledger records where the decision was
    made (comms' CHECK list already allowed `library`).
  - `core/src/route.rs`: `sjel://feed/library` opens the tab.
  - The reader now draws over Reading, Review and Library. The Feed keys act on the Reading list
    or in the reader, never on a list the open tab hides.
- **Not here**: the dashboard's Board and Calendar views, the metrics bar, the trip timeline, and
  the side peek (the reader replaces it).
- **Verified**: 1 new test (`library_filters_by_status_kind_and_query`) and 1 route assertion.
  Against the live archive: 986 entries, 963 new, 21 kept, 2 dismissed, matching comms. Keep,
  Dismiss and opening a row were not clicked, because each writes to the live ledger.
- **Target Files**: `apps/gpui/core/src/{shell,route}.rs`, `apps/gpui/src/feed.rs`

### ✅ Work Package 20: Inbox triage on the Mac (§1b step 3, last of three)
- **Status**: Shipped (2026-10-24). Mac only; the phone still captures and leaves triage here.
- **Already there**: send on (`r`), done, dismiss, the tag (`t`), and Apply all suggestions.
- **Shipped** (`src/inbox.rs`):
  - Edit: `↩` or Edit on the selected capture opens its text in a field. `↩` saves through
    `PATCH /comms/captures/:id`, leaving the field cancels. A price typed at the end is what the
    wishlist route reads (`capture::extract_price`), so the dashboard's separate price field is
    not needed.
  - Restore: `u` or Restore puts a done or dismissed capture back to new.
  - While a field is open, the list drops its `CaptureList` key context, so `j`, `e` and `d` type
    into the field instead of triaging.
- **Verified**: `typing_in_a_capture_edit_does_not_triage` in `src/workspace.rs`. It fails without
  the context change (the cursor moved to the next capture). Live inbox draws 10 new, 9 done,
  1 dismissed. Edit and Restore were not clicked, because each writes to the live inbox.
- **Target Files**: `apps/gpui/src/{inbox,workspace}.rs`

### ✅ Work Package 21: train connections (§1b step 2, second half)
- **Status**: Shipped (2026-10-24). Built in the shared travel surface, so a paired phone reaches
  transit through the Mac as well; checked on the Mac only.
- **Shipped**:
  - `core/src/travel/connections.rs` (new): transit's wire types (`capabilities/transit/src/travel.rs`),
    the query and its paths (one override: a typed phrase beats a preset), the presets
    (`transit::ranking::PRIORITIES`), the four sorts, and the readings: clock, duration, price,
    `NN% late`, first-leg delay, split saving, confidence and train-match notes.
    `Source::{stations, connections, split}` work through the shell or the device client.
  - `core/src/shell.rs`: `Shell::transit` reads `/transit/api/` paths only. `error_text` keeps a
    plain-text refusal (transit's `fail`, the proxy's 502) instead of only its status, for the
    shell and for the phone's `device::decode`.
  - `core/src/travel/navigation.rs`: `TravelRoute::Connections` for `/travel/connections`, which
    parsed as a trip with the id `connections` before.
  - `ui/src/travel/connections.rs` (new): From and To fields that suggest stations after typing
    pauses, a swap, date and time, and Search, which runs the direct search and the split together.
    Direct: four sorts, Rank by (My usual and the five presets, or a sentence), journey rows that
    expand to their legs and the ranking's factors. Split-ticket: direct against split price, the
    verdict, the confidence note, one card per ticket with its train match, and Book on bahn.de
    unless a ticket is for a different train. Opening the page starts transit on the Mac.
  - The trip library has Find a connection.
- **Not here**: the traveler profile dialog, the related-tools strip.
- **Found**: transit's searches fail upstream today. `/api/suggest` answers, but DB refuses
  journey requests from this Mac with `OPS_BLOCKED` on both backends (`dbnav` 452, `dbweb` 403,
  `capabilities/transit/src/hafas.rs`, `RailBackend`), so switching backends does not help. transit
  now names the block, answers 503, and sends nothing for 15 minutes after one. The dashboard's
  page drew its journey list only when the list was empty; fixed separately.
- **Verified**: 5 tests in `travel::connections` (sorts, query paths and validation, readings,
  split saving, a full transit split payload with the fields the page ignores) and 1 route
  assertion. `live_search_reads_transit` (ignored, needs this Mac) reached transit: station lookup
  answered, the search returned the HAFAS block above. The page draws on
  `sjel://travel/connections`; no search has rendered results yet, for that reason.
- **Target Files**: `apps/gpui/core/src/travel/{connections,navigation}.rs`,
  `apps/gpui/core/src/{travel,shell,device}.rs`, `apps/gpui/ui/src/travel/{connections,pages}.rs`,
  `apps/gpui/ui/src/travel.rs`

---

## 3. Next, before everything below (operator, 2026-10-23)

These come before the roadmap in section 4. The roadmap resumes when they are done.

1. **Ask and Search are one panel, as an overlay.** One input takes a search or a prompt. A mode
   switch offers *Auto*, *Ask* and *Search*; Auto decides from the text, and the panel shows which
   way it went so one keystroke can flip it. ⌘2 (Ask) and ⌘K both reach it.
   - Decided 2026-10-23: **overlay only for now**, in the ⌘K position. The docked right sidebar
     comes with item 2, not before.
   - Decided 2026-10-23: **Search is local and server, streamed.** Local results show at once:
     pages, destinations, and the rows the app already loads (tasks, notes, people, documents,
     inventory, finance). Server results append as they arrive, from a new shell `/api/search`
     that fans out to the capabilities. Today the shell has no cross-capability search (only
     transit's and interior's own `/api/search`), and the app holds no local index.
   - Done 2026-10-23, local stage: `core/src/search.rs` (the Auto rule, the ranking) and
     `src/finder.rs` (the overlay on ⌘K, which replaces the command palette and keeps its pages,
     actions and dashboard destinations as results). Projects, open tasks and notes come from the
     Mac's database. A question streams in place through the shared `Ask` session.
   - Done 2026-10-23, server stage: `GET /api/sjel-status/search`
     (`capabilities/sjel-status/src/status/search.rs`) reads the shared database read-only: events,
     people, documents, inventory, subscriptions, trips, and mail subject and sender. It returns
     five rows per source, each with a `sjel://` link. The finder appends them 180 ms after typing
     pauses.
   - Open: `LIKE` folds ASCII case only ("Ä" does not find "ä"), and document text is not
     searched, only titles. An FTS5 index fixes both when it matters.
2. **The right sidebar is where Sjel works in view.** A docked panel beside the page is the seat
   of the later adaptive UI, driven by what Sjel is doing: while it runs a task, the operator sees
   it search, open a page and change a value, live, instead of receiving only the result. Build
   the overlay from item 1 so the same view can dock here later, and so a stream of agent steps
   (the Ask turn's tool activity today) can drive navigation in the main column. The soundscape
   panel also slides in from the right edge; how the two share it is decided when this is built.
   - Decided 2026-10-24, and built for the phone in Work Package 16: **one edge with seats.** The
     frame (`ui/src/sidebar.rs`) draws the title, the seat chips and the close; a seat is the content
     a host hands in. The soundscape is one seat, the phone's own activity is the other, and the
     agent's step stream becomes a third rather than a rival panel. On the phone the frame is an
     overlay at 340 pt, because a phone has no width to dock beside a page; the same frame docks on
     the Mac, which is the next piece — the finder overlay (⌘K) is the view that docks into it.
3. **Soundscape, what the 2026-10-23 pass left.** ~~The phone has no player:
   `core/src/soundscape` is platform-free, so the iOS app needs an output device and a panel, not
   a second engine. The panel's own open state and the mixer disclosure are not kept across
   launches; nothing in the panel persists locally today.~~ Shipped for the phone in Work Package
   14: `AVAudioSourceNode` pulls the frames, and `SoundscapePrefs` keeps the open state, the mixer
   disclosure, the preset and the volume. The Mac still has its own panel; one panel for both
   belongs to item 2's right edge.
4. **The phone's next two moves (operator, 2026-10-24).** `Today` becomes the landing tab and
   `Projects` folds into it as a list, because `mobile_tab_bar` is full at five items and the phone
   opens on tasks more often than on the project list. Then `Inbox` — capture, writing to the
   replica through the outbox — because the calendar is the only write path today and a phone is a
   capture device first.
   - Done 2026-10-24: `Today` landed (Work Package 10), with `Projects` and `Settings` behind it.
   - Done 2026-10-24: `Inbox` landed (Work Package 12), in the fifth slot.
   - Decided 2026-10-24: the bar becomes `Today | Inbox | Calendar | People | Travel`, and
     `Settings` moves behind Today's header, with a banner on Today while the phone holds no copy
     or no pairing. Travel keeps a slot because it is the one page with a full map and a phone is
     the device you carry on the trip; Settings is opened when something is wrong, which is the
     moment a banner on the landing tab is the better affordance. Reversible in one line.
   - `Inbox` cannot queue offline before #244's blocked-write recovery: a capture that cannot
     reach the Mac keeps its text, says why, and retries when discovery finds the host again.
   - Done 2026-10-24: the soundscape player (Work Package 14) and local notifications (Work Package
     15, calendar entries and tasks; fare alerts stay unclaimed until the phone has a read path for
     them).
   - Still unclaimed after these: RoomPlan, and the right sidebar of item 2.

5. **RoomPlan comes to the phone, in three parts.** The operator's own phone is an iPhone 14 Pro, so
   `RoomCaptureSession.isSupported` is true and the scan is verifiable by hand on a real device —
   which is the one thing in this port that a simulator can never show. Two facts settled the shape:
   **RoomPlan is not Foundation Models.** RoomPlan is ARKit plus the LiDAR scanner and returns walls,
   openings and furniture as dimensions; Foundation Models is Apple's on-device language model, which
   has nothing to do with scanning (and whose Apple model needs an Apple Intelligence–eligible
   device). And the capture code **already exists**: `plugins/roomplan` on the retiring Tauri client
   holds 821 lines of Swift that build exactly the `roomplan-capture/v1` draft `libs/roomplan-contract`
   names and the Mac already reads. So this is a port, not a greenfield build, and the draft it
   produces is the one the review is tuned to.
   - Decided 2026-10-24 (operator): the phone captures **and reviews**. The review is a view over the
     capability's answer, not a second implementation: the comparison is `interior`'s,
     `roomplan::compare` against the flat's `room.toml`, and the phone draws what it returns.
   - Done 2026-10-24, part 1 — **the capture arrives**: `POST /api/roomplan/captures` in
     `capabilities/interior` takes the contract draft and the base64 USDZ, validates the draft with
     `roomplan_contract::CaptureDraft::validate` (the contract crate had no reader on this side until
     now), loads the flat's model *before* writing anything, stores
     `<assets_root>/captures/<flat>/<draft id>/{captured-room.usdz,observation.json}`, reads the file
     back through `roomplan::read_usdz`, and answers with `roomplan::comparison_json` — the document
     `interior roomplan --json` now also prints, extracted from `main.rs` so one shape has three
     callers instead of three copies. A capture the reader cannot parse is removed rather than left
     behind as the newest one. Tests: 1 in `api.rs` (a capture is stored and read back, or leaves
     nothing), 1 in `roomplan.rs` (the comparison names the capture as its caller does). interior 145.
   - Open, part 2 — **the Swift port**: `plugins/roomplan/ios/Sources/RoomPlanPlugin/RoomPlanPlugin.swift`
     moves into `apps/gpui/ios/host/` with the Tauri `Invoke`/`WKWebView` glue removed, writing the
     draft and the USDZ into the app's own storage with its sha256. Verified only on the 14 Pro.
   - Open, part 3 — **the review seat**: the right edge's third seat (`ui/src/sidebar.rs`), drawing
     the `comparison` document: wall height scan against `room.toml`, openings with widths, floor area,
     the objects, and the confidence; then accept, which posts to
     `/api/roomplan/revisions/{id}/review`. The simulator can show it over a fixture capture — the
     LiDAR line is the only part that needs the phone.

---

## 4. High-Priority UI/UX Improvements Roadmap

A substantial amount of UI/UX craftsmanship is needed to make the native app feel truly tactile, intuitive, and cohesive.

### 🚗 Travel & Map (`/travel`, `/map`) UX Overhaul (Highest Priority)
The current Travel page is functionally capable of reading trips and drawing OpenStreetMap/MapLibre tiles, but the experience is rigid and lacks critical operator workflows:

1. **Native Trip Creation & Booking Authoring**: shipped on the Mac in Work Package 17
   (2026-10-24). Editing the trip's own fields is still open.
   - *Was*: You cannot create a trip or add/edit a booking in GPUI; it redirects you to the web dashboard.
   - *Fix*: Add in-app modals:
     - `Create Trip`: Title, primary destination, start date, end date, tag.
     - `Add/Edit Booking`: Transport/hotel/activity type, confirmation code, carrier/provider, check-in/out times, attachment/link.
     - `Add Stage`: Origin, destination, departure/arrival timestamps, seat/cabin assignment.

2. **Unified Visual Itinerary & Timeline**:
   - *Current issue*: Itinerary is currently a plain text list separated from details.
   - *Fix*: Replace with a chronological timeline:
     - Day cards (Day 1: Oct 24, Day 2: Oct 25...).
     - Transport mode icons with color-coded transit pills (✈️ Flight, 🚆 Train / DB ICE, 🏨 Stay, 🚶 Walk, 🚗 Drive).
     - Visual connector lines between sequential stages with layover/connection duration badges.
     - Direct confirmation code copy button (`⌘C` to copy booking PNR).

3. **Interactive Map Polish (`ui/src/travel/map.rs`)**:
   - *Current issue*: Markers are simple numbers; tapping a marker does not pop up a card; stage lines are thin, direct straight lines.
   - *Fix*:
     - Stop pins with categorical glyphs (hotel bed, airport, fork/knife, camera).
     - Tap-to-inspect card: floating bottom sheet or sidebar flyout showing stop name, address, arrival date, and direct action to "Show in Apple Maps" or "Get Directions".
     - Stage curved geodesic paths connecting itinerary stops.
     - Auto-zoom "Fit Entire Trip" button with smooth camera easing.

4. **Live Fare Alerts & Sparpreis Lows**:
   - *Current issue*: Travel fare alerts (from `dashboard/src/lib/travel/fare-alerts.ts`) have no visual home in GPUI.
   - *Fix*: Add a "Fare Alerts" badge strip at the top of an upcoming trip showing price drop notifications, monitored DB train routes, and current lowest ticket price.

5. **Offline Readiness & Cache Health**:
   - *Current issue*: It is unclear whether a trip and its map tiles are cached before getting on an airplane.
   - *Fix*: Display an "Offline Ready" green check badge per trip, indicating:
     - All itinerary items & booking PDFs are saved in local store.
     - Map tiles for the trip bounds (zoom levels 10–15) are pre-fetched and stored on disk.

---

### 📄 Documents & PDF Reader (`/documents`) UI/UX
- **Server-Side Cursor Pagination**: Replace the 7,154-row full JSON payload fetch with chunked `cursor` paging (`/api/documents?cursor=...&limit=100`) so the view mounts instantly without network lag.
- **Multi-Page Reader Thumbnail Strip**: Add an expandable left-hand thumbnail rail to jump directly between pages of multi-page PDFs.
- **PDF Text Search**: Highlight matched query strings directly on the rendered PDF canvas.
- **Drag-and-Drop Ingest**: Allow dragging a PDF directly from macOS Finder into the GPUI window to trigger the import pipeline.

### 📅 Calendar (`/calendar`) UI/UX
- **Time-Slot Grid View**: Supplement the current day list with a 24-hour vertical timeline view (with current time indicator red bar).
- **Drag-to-Reschedule**: Keyboard shortcuts or drag interaction to shift an event by +/- 15 or 30 minutes.
- **Two-Way Sync Status Pill**: Clear indicator of Google / iCloud calendar sync recency and error states.

### 💰 Finance (`/finance`) UI/UX
- **Monthly Burn Visual Chart**: Add a lightweight sparkline/bar chart using GPUI canvas primitives for 6-month spending vs. income trends.
- **Inline Transaction Categorization**: Clicking a transaction's category opens a fast keyboard-navigable category picker (`F` or `Enter`) to reclassify on the fly.

### 👥 People (`/people`) UI/UX
- **Quick-Edit Drawer**: Edit contact notes, phone, email, and company directly without leaving GPUI.
- **Interaction History**: Display notes where this person was mentioned with timestamps.

### 📦 Inventory & Interior (`/inventory`) UI/UX
- **Wishlist Addition Modal**: Fast capture modal to add items with price range, target dimensions, and priority.
- **Interior Layout Viewer**: Interactive 2D floorplan canvas showing occupied room layout zones.

---

## 5. Mobile (iOS Standalone) Convergence Plan

The previous iOS client was wrapped via Tauri ([`dashboard/src-tauri`](../../dashboard/src-tauri)). The target is a 100% native GPUI client on iOS ([`apps/gpui/ios`](ios/README.md)):

```mermaid
flowchart TD
    subgraph iOSClient["Native GPUI iOS App (apps/gpui/ios)"]
        UI["GPUI Multi-Touch Views"]
        Store["Embedded SQLite / LibSQL Store"]
        Outbox["Offline Mutex Outbox Queue"]
        SyncMgr["Sync Engine (LAN / Tailscale)"]
    end

    subgraph Host["Mac Host / Daemon (LAN Listener)"]
        Bonjour["Bonjour Broadcast (_sjel._tcp)"]
        PinnedTLS["Pinned TLS HTTPS Listener"]
        CoreDB[("Live axon.db")]
    end

    UI --> Store
    UI --> Outbox
    Outbox --> Store
    SyncMgr <-->|Bidirectional Delta Sync| Store
    SyncMgr -->|Discovers| Bonjour
    SyncMgr <-->|Authenticated HTTPS| PinnedTLS
    PinnedTLS --> CoreDB
```

1. **Expanding iOS Tab Navigation**:
   - Current tabs: `Today` (the day off the copy, the tasks and projects off the replica), `Inbox`
     (capture into the replica, queued for the Mac), `Calendar` (replica, read and write; the Mac's
     copy until the first pull), `People` (contacts and places, read-only over the copy), `Travel`. `Projects` is the list a project row on Today
     opens, and `Settings` — the pairing — is behind Today's gear.
   - Add tabs:
     - ~~`Today`: Daily context timeline, focus tasks with large tap target checkboxes.~~ shipped in
       Work Package 10, which is also where the bar lost `Projects` and `Settings`.
     - ~~`Inbox`: Fast mobile capture field with tag selector (`#buy`, `#look`, `#go`, `#learn`).~~
       shipped in Work Package 12, which is what the fifth slot was kept for.
     - ~~`Calendar`: Mobile-friendly day view.~~ shipped in Work Package 7; a writing tab since Work
       Package 8, when the calendar joined the replica.
     - ~~`People` / `Places`: two registers, one tab.~~ shipped in Work Package 9.
   - **The bar is full at five.** `mobile_tab_bar` (`ui/src/nav.rs`) draws one row with
     `justify_around` and a 64 px floor per item: five items need 320 px, a sixth needs 384 px,
     which is wider than the 375 pt of the narrowest current iPhone. `Today` took the slot `Projects`
     gave up (Work Package 10) and `Inbox` takes the fifth, so a seventh page needs a different bar
     rather than a seventh item.

2. **Autonomous Offline Store**:
   - Bundle `rusqlite` / `libsql` directly inside the iOS app sandbox.
   - When offline, all reads hit the local SQLite cache.
   - All mobile writes (task completed, capture added, note edited) write to the local store and enqueue a row in the `outbox` table.
   - When `host/Discovery.swift` detects the Mac host on LAN or Tailscale, `sjel_gpui_core::sync` flushes the outbox and pulls down remote changes.

3. **Porting Tauri Native iOS Plugins to Swift / UIKit Bridges**:
   - **Apple RoomPlan LiDAR** (`plugins/roomplan`): Bridge `RoomCaptureView` in Swift to stream room dimensions and walls into GPUI interior layouts.
   - ~~**QR Code Scanner** (`tauri-plugin-barcode-scanner`)~~: shipped in Work Package 6
     (`apps/gpui/ios/host/PairScanner.swift`, `AVCaptureMetadataOutput`).
   - **Local Push Notifications** (`tauri-plugin-notification`): Use `UNUserNotificationCenter` for Sparpreis fare drops and calendar alarms without remote push servers.

---

## 5b. One app on every platform, and what it costs on disk (operator, 2026-10-24)

Not started. Each item is a work package for a later session.

1. **gpui-kit 0.8.0.** The app pins `gpui-kit = "=0.7.3"` and `gpui-pre = "=0.3.9"` in three
   manifests (`Cargo.toml`, `ui/Cargo.toml`, `ios/Cargo.toml`). crates.io has 0.8.0. The iPhone
   build also pins `gpui-mobile` to git rev `c24a1308e1`, which must resolve the same `gpui-pre`.
   So the upgrade moves all four pins together, or it splits Mac and phone onto two GPUI copies.
   What 0.8.0 changes is not read yet. Do it as its own commit, with `cargo test` on core and ui
   and `ios/run-sim` before and after (the `gpui` skill: no pin change inside an unrelated change).
2. **One UI crate, thin platform shells.** Measured 2026-10-24: `src/` (the Mac binary) holds
   41 files and 27,944 lines, `ui/` (shared with the phone) 23 files and 13,499 lines, and
   `ios/src` 2,664 lines. Most Mac pages (finance, inventory, systems, packs and more) live in
   `src/` and so are Mac-only by placement, not by need.
   - Move each page into `ui/` behind `Density`, the way Calendar, People, Today and Travel
     already are. The Mac and phone shells keep only windowing, navigation chrome and platform
     calls.
   - Platform code is already gated: `objc2-*` sits under `cfg(target_os = "macos")`, and 5 files
     use `target_os` or `native-maps` gates. Keep it that way. A platform call goes behind one
     small trait in the shell (keychain, open URL, audio out, notifications, scanner), never as a
     `cfg` inside a page.
   - Done 2026-10-24: twelve pages moved to `ui/` unchanged, apart from import paths: Finance,
     Inventory, Packs, Media, CV, Research, Backup, Systems, Setup, Self, Software and Mail. Each
     used only `data`, `theme`, `primitives` and `core`. Each old `src/` file now only re-exports
     its page, so the Mac shell did not change. `ui` checks for `aarch64-apple-ios`. The phone has
     no tab for any of them yet.
   - Still in `src/`: Documents (`pdf_kit`, PDFKit), Feed (needs Documents), Capabilities
     (`selection`, `workspace`), and the Mac wrappers `people_view`, `context_view` and
     `calendar_view` over pages that are already shared.
3. **Linux, Windows, Android.** These come after item 2. They mean a shell each, not a port each.
   - Linux and Windows: GPUI has desktop backends. The open work is the keychain
     (`libs/sjel-credentials`), the native map (`native-maps` is macOS only, the raster map is
     not), and packaging.
   - Done 2026-10-24: the Mac shell (`src/`) checks on Linux, in an OrbStack `rust:latest`
     container with CI's packages plus `libasound2-dev`. `pdf_kit` was the only blocker: Core
     Graphics is now gated to `Pdf` alone, and elsewhere `Pdf::open` answers
     `PdfError::Unsupported`, so Documents shows the extracted text. Not run on Linux yet, only
     checked. CI checks the shell on Linux and Windows. Every GPUI leg passed on 08a3347d and
     fails the build when it breaks.
   - Done 2026-10-24: on Linux, `KeychainProvider::system()` stores credentials in the Secret
     Service through `secret-tool`, with the same hex and the same service and account names as
     the macOS Keychain. No caller changed. A real round trip against gnome-keyring passes in the
     container (`the_secret_service_round_trips_a_credential`, ignored by default). A desktop
     without `secret-tool` (package `libsecret-tools`) gets a failed `start`, not a saved key.
   - Still open: a Windows keychain (no Windows CLI reads a stored credential, so this needs a
     crate), a PDF renderer off macOS, a launch on a real Linux desktop, and packaging.
   - Android: the phone's offline core (`core/src/{store,sync,mirror}.rs`) is platform-free. The
     open work is a host like `ios/host`, discovery, and the platform calls behind item 2's
     traits. Checked 2026-10-24: the pinned `gpui-mobile` rev `c24a1308e1` already has an Android
     backend (`src/android`, 347 `target_os = "android"` gates, an Android accessibility test), so
     no GPUI backend has to be written.
   - Done 2026-10-24: `gpui-targets` in `.github/workflows/ci.yml` checks the GPUI workspace
     natively on each platform: macOS plus the iOS target, and Linux, on core and ui, both
     strict; Windows and Android (NDK) report without failing until their first green run. A
     cross check from the Mac needs every target's C toolchain (`aws-lc-sys`), so each leg runs
     on its own runner. Checked locally: the iOS target builds. The other legs run first on push.
4. **Build footprint.** Measured 2026-10-24 (`du -sh`):
   - `~/.cargo-target`: 21 GB (debug 13, release 6.9, `aarch64-apple-ios` 1.2)
   - the repository's `target/`: 14 GB, of which 11 GB is `release/deps`
   - `~/.cargo/registry`: 6.2 GB
   - `ios/host/build`: 484 MB

   `tools/storage/storage` already measures all of this, and its policy protects what runs
   (`target/release` holds the service binaries, `~/.cargo-target/release` holds `sjel-mcp`).
   Use it. Do not add a second tool. 174 GiB was free on the same day, so this is hygiene, not
   an emergency.

   Done 2026-10-24:
   - The GPUI workspace gets the root's debug-info trim (`apps/gpui/Cargo.toml`,
     `[profile.dev]`). It never inherited it, so every GPUI dependency carried full debug info.
     The saving is not measured yet. The first debug build after this rebuilds GPUI, and the
     old artifacts stay in `~/.cargo-target/debug` until the debug-cache class prunes them.
   - The GPUI workspace and its installer share one target dir. `install` exported nothing and
     `src/update.rs` passes only `PATH`, so the in-app Rebuild built a second release tree in
     `apps/gpui/target`: byte-identical binaries eight seconds apart, and a cold build whenever
     the two paths alternated. `install` now keeps `CARGO_TARGET_DIR` when set and otherwise uses
     `$HOME/.cargo-target` when it exists, and `cargo metadata` follows it for the bundle path.
     Measured in the shared tree, warm release builds: 0.2 s unchanged, 11.8 s for one file in
     `src/`, 17.1 s for `ui/`, 21.1 s for `core/`. A cold build of the whole tree is about 3 min.
     Left alone: `install` passes `SJEL_BUILD_COMMIT` and a bare `cargo build` does not, so
     alternating between the two recompiles the binary (about 12 s).
   - Both dead build trees were reclaimed 2026-10-24, 3.6 GB in total: `apps/gpui/target`
     (1.9 GB) with `storage reclaim --repo --target`, and the repository's `target/debug`
     (1.7 GB from 2026-10-22, unreachable because the shell sends every debug build to
     `~/.cargo-target/debug`) with `cargo clean --profile dev --target-dir target`.
   - The soundscape's half-minute quality test is `#[ignore]`d: 20.8 s of every core test run.
     It takes 2.4 s with `--release --ignored`.

   Open:
   - The root workspace is release-built twice: `./target/release` (12.2 GB, the services) and
     `~/.cargo-target/release` (7.0 GB, `sjel-mcp` and every interactive build). Decided
     2026-10-24 (operator): unify into one location, in a session of its own. It touches every
     `service.toml` `command` and the `sjel-mcp` path.

   Decided 2026-10-24 (operator), no work left:
   - The overlay policy keeps its 50 GB cap on `rust-debug-cache`.
   - `alias cargo='taskpolicy -b cargo'` is off (`~/.zshrc`), so builds run at full speed
     everywhere. It had cost about 3× the wall time for about a fifth of the power (measured
     2026-10-08), and agent sessions ran under it too.
   - Release profile: `strip`, and `lto = "thin"` only where a measurement shows the runtime
     gain is worth the build time. The services are I/O-bound, so expect little.
5. **Efficiency and simplicity.** Two known items:
   - Documents still fetches 7,154 rows on mount (§4, cursor paging).
   - The finder's server search uses `LIKE`, which folds ASCII case only (§3 item 1, FTS5).

   Beyond those, run `simplify` over each page as it moves into `ui/` (item 2). A move is the
   cheapest moment to delete what the second client does not need.

---

## 6. Session Collaboration Guidelines

When picking up a work package:
1. **Never touch or stage files under `capabilities/documents/src/`** (reserved for active document classifier session).
2. **Always run unit tests before and after changes**:
   ```sh
   cargo test --manifest-path apps/gpui/core/Cargo.toml
   cargo test --manifest-path apps/gpui/Cargo.toml
   ```
3. **Deploy and verify desktop builds locally**:
   ```sh
   apps/gpui/install
   ```
4. **Follow `attention-control` doctrine**:
   Keep UI clean, low-clutter, fast, keyboard-first, with zero generic AI purple gradient aesthetics.
