# GPUI: one native app

The native GPUI client ([`apps/gpui`](../../apps/gpui/README.md)) is the one Sjel app on macOS and
iOS, with Linux, Windows and Android later. A rule the Mac and the phone share goes into a
capability or `apps/gpui/core`. The renderer for each platform is decided in §11: GPUI Fast on the
desktop, and a trial of NSH on the phone.

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
| 5 | `/documents` | Ported | `Documents` | behind `Today` | `documents` |
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
| Documents | Shared + mobile shortcut | `ui/src/documents.rs` + Core Graphics `ui/src/pdf_kit.rs` on macOS and iOS. 1-tap shortcut on Today, single-pane mobile reader with pinch zoom. |
| Mail | NSH triage screen + desktop | `ui/src/nsh_mail.rs` on iOS (`Tab::Mail`) and Mac NSH trial (`SJEL_NSH_TRIAL=mail`), plus `ui/src/mail.rs` on Mac desktop. |
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

1. Done: `Documents` (`ui/src/documents.rs`) and Core Graphics PDF rendering (`ui/src/pdf_kit.rs`)
   on the Mac. The phone no longer reaches this GPUI page: its Documents is the NSH reader.
2. Built as the NSH reader (§11): the phone lists documents from `GET /api/documents` in pages of
   500, and opens the original in PDFKit at fit width after checking its SHA-256 against the
   library's. Open: cursor paging (§7).
3. Built (§11): "Keep offline" stores the original and its detail in a folder with
   `NSFileProtectionComplete` (rule 2), and kept documents list and open without the Mac.
4. Then [#245](https://github.com/larsboes/Sjel/issues/245) (filing and side-by-side compare),
   [#247](https://github.com/larsboes/Sjel/issues/247) (reviewable proposals) and
   [#246](https://github.com/larsboes/Sjel/issues/246) (durable duplicate and split decisions).

Done when a real multi-page PDF opens at fit width on the phone, a kept document opens with the
Mac unreachable, and the original's SHA-256 on the phone equals the library's.

### Mail

1. Built as an NSH screen (`ui/src/nsh_mail.rs`, §11): status and category filter chips, virtual
   list of `MailProposal`s ordered by category/relevance/date, and a markdown thread reader via
   `GET /comms/content/mail/{id}`.
2. Built: triage actions (`Dismiss`, `Waiting`, `Archive`, `Trash`, `Secret (c3)`, category moves,
   model verdict `Accept`, and Gmail job `Retry`/`Cancel`) run through `POST /comms/triage/bulk`
   (`Source::mail_action`), with inline confirmation before archive, trash or class-raising moves.
3. Built: offline caching of opened mail threads and detail (`MailOffline::cache_thread`), and outbox
   queueing (`MailOffline::queue_action`, `replay_outbox`) when the Mac is unreachable, replaying
   automatically on reconnect. Open: attachment handoff into the document reader.

Done when the phone triages ten mails with the Mac reachable and the Mac shows the same state, an
action taken offline applies once on reconnect, and an attachment opens in the reader.

### Calendar

1. Day focus and week views use a 24-hour time axis, duration-sized events, overlap columns,
   a now line, and a separate all-day strip. Shared placement rules live in `apps/gpui/core`.
2. Long-press and drag moves an event in 15-minute steps. The write goes through the replica, the
   same path as the New form.
3. A status mark shows the last successful Google and iCloud sync, and an error if one failed.
4. Direct time editing includes resize, a keyboard-accessible edit action, and Undo. A selected
   empty slot prefills its time. Compact agenda rows disclose location, notes and source on
   selection; Mac details use an inspector and phone details use a sheet.
5. Availability mode shows free gaps between commitments, with explicit tentative styling.
   A gap can create an entry or start travel planning. Month overflow opens a day agenda, and
   multi-day trips render as continuous spans. Decided 2026-11-05.

Done when a moved event shows the new time on the Mac after the next sync, and the mark matches
the Mac's last sync time. Calendar UX is done when overlap, overnight, all-day, dense-month,
keyboard, drag, resize and Undo scenarios pass; Day focus and availability work on Mac and phone;
and rendered views have been checked in both themes. Implemented, awaiting native acceptance:
Day and Week placement, quarter-hour move and resize, time editing and Undo, compact details,
availability-to-entry and desktop travel drafts, month overflow, all-day spans, Google/iCloud/replica
sync-status marks, and cross-capability navigation from Calendar to Comms/Mail and Travel. Verified: 32 core calendar and replica sync tests, 13 calendar UI tests, desktop
and iOS simulator type checks. The UI tests cover desktop drag, keyboard editing, a 375-point phone
layout, and desktop/mobile sync status marks and error handling. Provider health is not exposed by the
export-opt-in endpoint, so desktop Google/iCloud status remains unknown rather than healthy; a failed
Google status request is shown as an error. The time-undo test invokes the action
directly and only checks the unavailable-shell failure; keyboard dispatch and successful restoration
are not covered. Replica tests cover queued move,
resize, restoration, and sync replay where a moved event updates the Mac and timestamps the sync mark.
Long-press touch capture, quarter-hour move preview, edge resize, cancellation and invalid resize rejection
have a headless gesture test. Open: native phone gesture acceptance on live device, and scroll-cost
measurement. Native desktop Day, Week, Month and availability were reviewed in both themes through app control.
Month overflow below the window was found and has a bounded-viewport regression test; native verification of
that repair remains open. The offscreen PNG example
(`cargo run --manifest-path apps/gpui/Cargo.toml -p sjel-gpui-ui --example calendar-preview -- /tmp/calendar.png week light`)
compiled, but sandboxed AppKit pasteboard access failed before rendering. The app-control
connector can inspect the installed app in the same session. Device checks have not passed.

Desktop bundle replacement preserves the previous app until the incoming bundle moves into
place and restores it if that move fails. Four checks cover preservation, an ordinary install
failure, a partial-destination rollback, and successful replacement (`bash tools/replace-directory.test.sh`). The installer can
package an existing release binary with `SJEL_DESKTOP_PREBUILT` and its matching
`SJEL_DESKTOP_PREBUILT_COMMIT`; failed staging is retained for recovery. LaunchServices
registration failure is reported separately from installation.

### Travel

0. **A destination has a coordinate.** Shipped: `crates/trips/src/place_resolve.rs`
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
6. **Trip selection and deletion.** Shipped: a library row opens the trip overview through
   `Travel::select` (`ui/src/travel/library.rs`), and the overview's foot now carries the
   two-press "Delete trip" action, whose removal path (`Remove::Plan`, `Work::DeletePlan`) was
   previously reachable only from item and stage rows (`ui/src/travel/overview.rs`). The delete
   reads the plan's `updated_at` and sends it back as `expected_updated_at`, so a trip edited
   elsewhere after it was opened is refused (`core/src/shell/people_calendar.rs`). Not run: the
   manual check that a click opens the overview from every section, that one press arms the
   delete and a second deletes the trip, and that a stale revision is refused. No headless test
   covers either path: the GPUI test scheduler treats the details read that `select` starts as
   nondeterministic activity.

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

The dashboard and its desktop shell are deleted. GPUI has no browser handoff: a path no page
answers opens the page of its first segment. No rule lives only in TypeScript:
`merge-candidate` is `crates/entities/src/duplicates.rs`, `who-is-around` is
`core/src/travel/around.rs`, `mail-model-proposal` is `ModelVerdict::proposes_change` in
`core/src/mail.rs`. The previous client's Foundation Models feature is retired.

The former gates are now open checks. None of them ran:

1. Work Packages 23 (People writes) and 24 (Finance import) pass the manual acceptance test.
2. The new views: People → Duplicates, a trip's "Who is around", the mail model verdict with
   Accept, and Retry or Cancel on a stopped Gmail action.
3. Fare alerts (Work Package 27) on a device, and the legacy RoomPlan drafts on a phone that holds
   existing capture exports. Remove the retired iOS app build (`com.larsboes.sjel`) if it is still
   installed.

Open work:

- [ ] (TODO: deferred, not in today's scope) Screenshots of the app for the public landing page ([`restructure.md`](restructure.md#4-the-public-page)).

Shipped (awaiting manual check in the running app):

- `/feed/[id]`: queueing and running a cloud job (`/comms/content/cloud-jobs`), and lowering a
  data class with a required rationale (`apps/gpui/ui/src/feed.rs`).
- `/inventory`: state history (`GET /inventory/api/items/{id}/state`), state transitions, and layout
  impact preview (`POST /interior/api/items/{id}/impact`) in the item edit form
  (`apps/gpui/ui/src/inventory.rs`).
- Stay ranking board: candidates stored as `stay` items on the trip plan and ranked through
  `PATCH /trips/api/plans/{plan_id}/items/{item_id}/fields` (`apps/gpui/ui/src/travel/pages.rs`,
  `apps/gpui/core/src/travel/edit.rs`).

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
| 26 | Inventory and wishlist create, edit with the read revision, state changes, state history (`GET /inventory/api/items/{id}/state`), and layout impact preview (`POST /interior/api/items/{id}/impact`). Inventory stays the only writer. | Both views refresh without restart. A stale edit is refused and the entry reloads. State history and layout impact preview render in the edit form. |
| 27 | Low-fare alerts on the iPhone: rule in `core/src/travel.rs`, unseen `sparpreis-low:` notes through the local notification host, ten-minute throttle, works from saved trips offline | A new low notifies once. A repeat check does not. It works offline. |
| 28 | Wish comparison. Built: a wish group's candidates carry numeric specs (`inventory specs`, `PUT /api/items/{id}/specs`), the group carries needs (`inventory needs`), and each candidate has dated price observations, one per source and day (`inventory price`, `POST /api/items/{id}/prices`). `GET /api/wish-comparison` reads it all. The daily job [`wish-prices`](../../capabilities/wish-prices/README.md) runs `inventory prices refresh`. It reads only geizhals product pages, never a search page, which geizhals' `robots.txt` disallows. It identifies itself as Sjel, never as a browser, and takes the page's `og:price:amount`. The first price at or below `cost_max_cents` is the alert. The wishlist tab shows, for the selected group, price against a chosen spec with the need and limit lines, a price history, and per candidate the latest price, fit to needs, and € per unit of each spec. The rules are in `core/src/inventory_compare.rs`. A wish group marked as alternatives (`inventory alternatives`, the "Pick one" button) counts only its cheapest member in every budget. | Built: a daily run adds one observation per candidate, and a second run on the same day adds none. A search URL is refused as a source. A price at the limit alerts once (`crates/inventory/tests/compare.rs`). Not run: the manual check in the app with three priced candidates. |
| 30 | Task statuses and the cross-project task table. A task is `open`, `doing`, `waiting`, `blocked`, `done` or `cancelled` (`TASK_STATUSES` in `crates/projects-api`). `done` and `cancelled` set `done: true`; the others set `done: false`. The All Tasks table filters by status (`Active` hides closed tasks), sorts by any column, groups by project, and edits status, due and priority in the row. A click on a title opens a side panel for status, priority, due, context, blocked-by and the remaining frontmatter, with a link to the note. Open: the phone replica shows statuses but refuses a status, context or blocked-by write, because `core/src/sync.rs` replays only done, priority and due. | Built: `tasks::tests` and `projects::tests::all_tasks_panel_opens_on_a_task_and_draws_beside_the_table` in `sjel-gpui-ui`, and `set_property_checks_schema_keys_and_status_drives_done` in `sjel-projects`. Not run: the manual check in the installed app, which sets each status, sorts, groups and edits from the panel. |

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
   since the rename change. Rename spans `crates/projects/src/{ops,record,store}.rs`,
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
  (`crates/sjel-status/src/status/search.rs`), five rows per source.
- **Open: search quality.** `LIKE` folds ASCII case only ("Ä" does not find "ä"), and document
  text is not searched, only titles. An FTS5 index fixes both. Do it when a missed result costs
  something.
- **Right edge.** One frame (`ui/src/sidebar.rs`) with seats: soundscape, the phone's activity,
  and the agent run. On the phone it is a 340 pt overlay. On the Mac it docks.
- **Open: Sjel works in view.** The later adaptive UI: while the agent runs a task, the main
  column follows its steps (search, open a page, change a value). The agent's step stream is the
  input. Not designed yet.

## 6. One UI crate, every platform

1. **GPUI pins.** Done 2026-11-07: `gpui-kit =0.9.0` and `gpui-pre =0.3.11`
   (`apps/gpui/Cargo.toml`) with the `gpui-mobile` fork `larsboes/gpui-mobile` rev `bbedd2d`
   (`ios/Cargo.toml`), one matching set. The three move together by hand in one commit, with
   `cargo test` on core and ui and `ios/run-sim` before and after. Dependabot ignores them
   (`.github/dependabot.yml`). Open: a return to the upstream rev once it carries 0.3.11.
   `longbridge/gpui-mobile` #46 makes the same change and was checked on Android. #47, from the
   fork, duplicates it.
2. **Pages into `ui/`.** Twelve Mac pages moved to `ui/` on 2026-10-24 (`b2fb5246`, `2ce818de`), and
   seventeen more modules (`documents`, `pdf_kit`, `feed`, `scout`, `clipper`, `context`,
   `people::page`, `calendar::page`, `inbox::triage`, `import`, `music`, `ask`, `capabilities`,
   `docs`, `accounts`, `selection`, `viz`) moved on 2026-11-03. The phone shell is split by
   seam: `host.rs` (the Swift boundary both ways), `events.rs`, `nav.rs`, `edge.rs`,
   `pairing.rs`, `launch.rs`; `lib.rs` keeps the view, its construction and `render`. The agent
   controller moved to
   `ui/src/agent_seat.rs` over `Host` (`core/src/agent.rs`); the Mac finder and the phone edge
   draw the same seat. Still in `src/`: desktop shell
   orchestration and host window management (`workspace.rs`, `finder.rs`, `settings.rs`, `soundscape.rs`,
   `sidebar.rs`, `header.rs`, `update.rs`, `home.rs`, `agent_run.rs`, `audio.rs`). A page reaches the
   platform through GPUI's own API (`cx.open_url`, the clipboard, `prompt_for_paths`). A service
   only one shell has reaches the page as a handler the shell injects, as
   `InventoryPage::set_cache_preview_handler` does for the RoomPlan preview (`src/roomplan.rs` on
   the Mac, `host/` on the phone). Never a `cfg` in a page. No shared trait: each service has one
   implementation per shell and no shared caller. The two `audio.rs` files are two device
   backends over one engine (`core/src/soundscape`): cpal pushes on the Mac, `host/Audio.swift`
   pulls on the phone. Run `simplify` on each page as it moves.
3. **Phone tabs for moved pages.** The phone bar is five items (`Today | Mail | Calendar | Travel |
   Agent`, `ios/src/lib.rs` and `ui/src/nav.rs`). Inbox capture lives on Today; Documents, People
   and Places open via 1-tap shortcuts and finder. A sixth tab needs a different bar, not a sixth
   item. The phone opens the Mac's destinations (`route::Target`) through
   `phone_screen` and one `select_tab` (`ios/src/nav.rs`); a page it does not draw says it opens
   on the Mac. An unknown path falls back to its first segment's page on both devices
   (`route::first_page`). Open: the host does not register the `sjel://` scheme yet, so only a
   reminder's link reaches `open_link`; that needs `CFBundleURLTypes` in `host/Info.plist` and an
   `onOpenURL` call into Rust. Next on the phone: Inventory and Wishlist, then People writes (D3 in [`PLAN.md`](../../PLAN.md)).
4. **Linux, Windows, Android.** The Mac shell checks on Linux (`4cd60961`), and CI checks every
   leg (`578e6948`). Linux credentials use the Secret Service (`37adf658`). Open: a Windows
   keychain crate, a PDF renderer off macOS, a launch on a real Linux desktop, packaging, and an
   Android host like `ios/host`. The pinned `gpui-mobile` already has an Android backend. If the
   NSH trial in §11 passes, the Android phone app comes from NSH's Android backend instead.
5. **Lints.** Clippy is clean for `apps/gpui` at `--all-targets`
   (`sjel cargo clippy --manifest-path apps/gpui/Cargo.toml --all-targets --locked`). The
   remaining warnings were a manual counter loop, manual prefix slicing, a reversed-key sort,
   `too_many_arguments`, `type_complexity`, a needless borrow and an ambiguous `into()`. The
   argument and tuple bundles are now named types: `PlacementUpdate` (`core/src/interior.rs`),
   `InteractiveStatCard` (`ui/src/primitives/cards.rs`), `CapabilityRow`
   (`ui/src/capabilities.rs`), `ImportDerivedKey` (`ui/src/finance/import.rs`) and the render
   cache aliases in `ui/src/docs/diagrams.rs`. Format a change with
   `rustfmt --edition 2021 <file>`: a bare `rustfmt` sorts imports under another style edition
   and rewrites every `use` it touches, which `cargo fmt --check` then rejects. No gate covers
   formatting in `apps/gpui`, because the repository's format step runs at the root workspace
   ([`tooling.md`](tooling.md#open)). `apps/gpui/src/sidebar/mod.rs` and
   `apps/gpui/ui/src/theme/settings.rs` were already unformatted at `d07edc16` and stay out of
   this change.
6. **Failing desktop test.** `failed_task_write_restores_state_when_the_shell_is_unreachable`
   (`src/workspace/mod.rs`) fails at `315fa7ee` and after: the task stays done after a write to an
   unreachable shell. Not investigated; the write runs on the HTTP runtime, which
   `run_until_parked` does not wait for, so the test may assert before the restore lands.

## 7. UI and UX roadmap

Polish does not block retirement. The write paths do. The phone priority areas are in §2. Order
inside this list is by use.

**Component boundary (decided 2026-11-08).** Do not build a separate base widget kit from scratch.
Use a three-layer split across `gpui-fast` and `nsh`:

1. **Kit primitives wrapped in `ui/src/primitives/`:** Reuse `gpui-kit` on the desktop and
   `nsh-base` on NSH screens for tables (`DataTable`, `sortable_table_header`, `tabular_cell`),
   Markdown/HTML readers (`TextView` with `FrontmatterPlugin` and `text_view_style`), rich editing
   (`nsh_base::rich_editor`), and date/time controls (`Calendar`, `DatePicker`, `TimeField`).
2. **Native OS views for heavy media:** Embed platform views (`PDFKit` `PDFView` as `sjel.pdf`,
   `MapLibre`) where pixel-level reimplementation degrades text selection, zoom or search.
3. **Custom views only for Sjel-specific data shapes:** Keep shared layout math in `apps/gpui/core`
   and hand-built views only for the 24-hour Calendar timeline and availability grid, cross-capability
   join inspectors, and the block note editor.

- **Documents.** Built (2026-11-08, `de389e7d`, `49aaf9f9`): NSH Documents (`ui/src/nsh_documents.rs`) loads
  one 500-row page on mount with on-demand paging (`fetch_page`), in-place sort chips (`Newest`,
  `Name A–Z`, `Category`, `Pages`), and `nsh_base::text_view` selectable Markdown/links; Mac
  Documents (`ui/src/documents/`) renders extracted text through `gpui_kit::component::text::TextView`,
  adds 500-record cursor pagination controls (`prev_page`, `next_page`, `page_offset`, table toolbar count),
  a quick page jump strip (`Reader::jump_pages`, `page_jump_strip`), a collapsible low-resolution
  page thumbnail rail (`Reader::thumbnails`, `THUMB_WIDTH`, `request_thumbnails`), in-PDF page
  text search (`search_extracted_pages`, `page_search_hits_strip`) with matched-page badges and
  snippet previews, side-by-side filing compare workbench (#245) with dual reader canvases (`filing_compare_id`,
  `open_compare_document`, `adopt_compare_category`), `DocSort` multi-column sorting and resizable columns, and direct Finder
  drag-and-drop (`ExternalPaths` / `on_drop` / `drag_over`) plus `+ Import…` file picker upload
  (`Documents::import_paths`, `Documents::prompt_import`) sending raw file bytes to
  `POST /documents/api/documents`.
- **Markdown & Rich Text.** Built (2026-11-08, `de389e7d`): `doc_view::render_markdown` and
  `ui/src/docs/` use `gpui_kit::component::text::TextView` with `FrontmatterPlugin`,
  `MermaidPlugin` (`ui/src/docs/diagrams.rs`), and Sjel's `Palette`-mapped `text_view_style`,
  replacing the duplicate hand-rolled Markdown/table/block parsers in `ui/src/research/articles.rs`
  and `ui/src/docs/render.rs` while preserving internal doc-slug routing and `#anchor` scrolling via
  `TextViewState::reveal_range`.
- **Tables.** Built (2026-11-08, `a2f5647e`, `ed998522`): `SortableColumn`, `SortOrder`,
  `sortable_table_header`, `resizable_table_header`, `cycle_column_width`, and `tabular_cell` in
  `ui/src/primitives/tables.rs`, wired into All Tasks, All Notes, Project Tasks
  (`ui/src/projects/tables.rs`), Documents (`ui/src/documents/mod.rs`), Finance Transactions and
  Import Candidates (`ui/src/finance/transactions.rs`, `ui/src/finance/import.rs`), and Inventory
  Owned (`ui/src/inventory/owned.rs`) with per-column sorting and interactive column resizing while
  keeping `virtual_list` group headers and inline row editors intact.
- **Calendar.** Built (2026-11-08, `102dfe5f`): Keeps the 24-hour timeline, quarter-hour drag/resize and
  availability grid in `ui/src/calendar/`, extends the 24-hour time grid to a 7-day week view (`View::Week`,
  `time_grid`) with duration-sized events, overlap columns (`timeline::blocks`), all-day spans across the week,
  cross-day drag-and-drop rescheduling with drop preview ghost, now-line on today's column (`calendar-now-line`),
  compact inline title/time formatting for short events (<30 min), and wires `gpui_kit::component::date_picker::DatePicker`
  and `gpui_kit::component::time_field::TimeField` into the event composer (`ui/src/calendar/page.rs`,
  `ui/src/calendar/types.rs`, `ui/src/calendar/grid.rs`) with automatic end-date clamping when the
  start date moves past the end date.
- **Finance.** Built: the overview cards show the newest month instead of the
  all-time sum, a six-month income and spending chart, and a holdings panel with each price's
  source, age and FX rate. A keyboard category picker in Import & Review: typed words filter
  the known expense accounts, ↑↓ move, Enter applies and selects the next uncategorized
  expense. Paging for `GET /api/import/candidates` is dropped: measured 2026-11-06, 1,470
  candidates (859 KB) answer in 0.14 s, against the app's 10 s timeout.
- **People.** Interaction history: notes that mention the person, with dates.
- **Inventory.** State history and layout impact preview (from Work Package 26).
- **Travel, Tasks, Notes & Feed UI/UX upgrades (2026-10-28).**
  - *Shared Markdown Block Reader:* `render_markdown` and `styled_inline` in `doc_view.rs` wired across Feed reader and Travel agenda.
  - *Travel Readiness & Day Filter:* Coverage readiness strip (nights, missing booking codes), quick day pill filter (`All days | Day N`), direct/buffer transfer badges.
  - *Projects & Tasks Power-Up:* 1-click due preset chips (`Today`, `Tmrw`, `+7d`, `Clear`), subtask progress badges (`done/total`), smart task input syntax (`!1`-`!3`, `@today`, `@tomorrow`, `@YYYY-MM-DD`).
  - *Notes & Editor Outline:* Note kind filter chips (`All | Hubs ★ | Notes ≡`), 2-line snippet preview under titles, editor task completion badge, and clickable heading jump outline (`§ H1/H2...`).
  - *Feed & Inbox Triage:* Feed row evaluation score badges, inline 1-click triage buttons (`Keep | Dismiss`), formatted Inbox capture cards with status pills and tag badges (`#task`, `#idea`, `#place`, `#price`).

## 8. Rules for a session that takes a work package

1. Do not touch or stage files under `crates/documents/src/`. Another session owns them.
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

GPUI Fast against `gpui-pre`, release build, 2026-11-07 (`apps/gpui/bench-scroll <page>`, CPU
from render to paint in ms, frames over 8.3 ms). `gpui-pre` 0.3.11 shows its second run: the first
painted 61 and 81 frames on systems and documents and is dropped. GPUI Fast 0.1.13 shows both runs.

| page | `gpui-pre` p50 / p95 / over | GPUI Fast p50 / p95 / over |
|---|---|---|
| systems | 2.01 / 2.32 / 0 | 1.16–1.20 / 1.29–1.59 / 0 |
| inventory | 3.11 / 4.41 / 0 | 2.23–2.24 / 3.14 / 0 |
| documents | 3.57 / 8.65 / 25 | 3.27–3.55 / 6.49–6.76 / 4–6 |
| packs | 3.67 / 10.34 / 26 | 4.89–5.10 / 5.52–5.71 / 0–1 |
| feed | 4.37 / 7.66 / 7 | 2.71–2.87 / 5.42–5.80 / 5 |

GPUI Fast lowers p50 by 28 to 41 percent on systems, inventory and feed, and cuts the frames over
budget from 58 to at most 12. Packs is the exception: p50 rises by about a third while p95 halves.
The 73 to 88 percent in the GPUI Fast README are not reproduced here. Systems no longer misses
the budget on either build, so the 2026-11-02 table above is out of date for that page.

### Done

| # | What | Result |
|---|---|---|
| E1 | Feed's Reading list draws one row per item on screen: `ListState` and `list()` in `ui/src/feed.rs`, the order cached in `Feed::reading_order` and rebuilt only when the entries, the filter or a status move. Feed owns its scroller and every other tab in the page keeps one of its own. `j`/`k` reveals the cursor's row. | 71.7 → 1.7–3.7 ms p50 for 362 rows, of which about 15 are drawn. |
| E2 | Shared `virtual_list` helper in `ui/src/primitives.rs` wraps `ListState`, scrollbar, and `flex_1 + min_h_0`, wired across Feed Reading, Feed Library, and Documents. | Three lists share `virtual_list`. |
| E3 | Filter, sort, count pills, and `Board::from_records` removed from `render()` across `ui/src/projects.rs`, `ui/src/feed.rs` (`sync_reading`, `sync_library`), and `ui/src/documents.rs` (`sync_order`). | Unchanged frames allocate no per-row filter/sort vectors. |
| E4 | Poll hygiene: `UPDATE_CHECK` runs at most every 5 minutes, and `Remote::settle_changed` + `data::read_quiet` / `read_repo_quiet` notify only when the value or error moved. | Idle polls and unchanged SSE snapshots wake no repaint. |
| E5 | `SystemsPage` beats (`MACMON_BEAT`, `AGENT_BEAT`, storage, updates, findings, runtime) use `data::read_quiet` over `PartialEq` models. | Beats reporting an unchanged reading paint nothing. |

### Open (E2 remaining non-uniform pages)

| page | what it draws | state |
|---|---|---|
| Inventory, Interior | grouped cards in one scroll container | Open. They build cards rather than uniform rows, so a conversion needs a looked-at window; not done blind. |
| Packs | a matrix with a pinned header and a horizontal scroller | Open, and last: 5.7 ms with 2 of 238 frames over budget. |

## 10. Dev and prod

The app runs against the live node (prod) or the demo node (dev). The dev node is the seeded
stack in a container (`tools/demo-shots`, [`demo/README.md`](../../demo/README.md)), and its data
is synthetic only. A copy of prod data is never a dev source: agents work on dev data.

Built:

- `sjel demo up | app [page] | down`. `up` builds, starts and reseeds the node on port 18082.
  `app` starts the Mac app built from this checkout against it, with its own HOME and a fresh
  store snapshot on each launch.
- One resolved environment: `core/src/environment.rs` reads `SJEL_STATUS_URL` and
  `SJEL_ENV`, and every endpoint reader goes through it. The store paths keep their one reader
  each (`core/src/data/repo.rs`, `core/src/data/board.rs`).
- Dev badge: a window started by `sjel demo app` (`SJEL_ENV=dev`) shows `DEV NODE` in the
  header. Checked by a screenshot of the Settings page on the demo node. Not built: a distinct
  accent.
- Switch: Settings › Connection & Sync › Node. "Open on dev node" runs `tools/demo-shots app`
  and quits; it reports the script's error when the demo node is down. "Back to live" opens
  `dev.sjel.desktop` and quits. Not checked: either click.

Open:

1. **Seed coverage for Priority 1.** Documents are seeded: four PDFs and two text files
   (`tools/lib/demo-data.ts`, `documents`). Agent sessions are unverified. Not checked: the
   Documents page and the reader in `sjel demo app`.
2. **Phone on the dev node.** Needs the LAN listener and the `devices` capability in the demo
   stack (also blocks the iPhone screenshots in [`restructure.md`](restructure.md)).

## 11. UI stack

Decided 2026-11-07: GPUI Fast draws the Mac app, and [NSH](https://github.com/longbridge/nsh)
draws new phone screens with UIKit views. GPUI draws every pixel itself, which suits the desktop.
On the phone it has to rebuild text selection, input methods and accessibility that UIKit already
has. NSH's GPUI backend runs on the same `gpui-fast` the Mac app uses, so one NSH screen also
opens inside the Mac app. Evidence: [`research/ui-stack.md`](../../research/ui-stack.md).

### Built

| What | Where | Commit |
|---|---|---|
| Pins: `gpui-kit` 0.9.0, `gpui-pre` 0.3.11, `gpui-mobile` fork | §6 item 1 | `d2970986` |
| Crates reach GPUI only through the Kit (`extern crate gpui_kit as gpui;`) | each crate root | `21966ff8` |
| GPUI Fast is the Mac default; `--no-default-features` builds `gpui-pre` | `apps/gpui/Cargo.toml` | `fdf3f6cb` |
| Frame cost on both renderers | §9 | `c28ffb23` |
| NSH Documents reader, one screen for phone and Mac | `ui/src/nsh_documents.rs` | `62145b47` |
| `device::Source`: the Mac's shell or the phone's client, one read path | `core/src/device.rs` | `62145b47` |
| Documents on the phone is the NSH reader; "Back" returns to GPUI | `ios/src/nsh.rs`, `nav.rs`, `launch.rs` | `a71af504` |
| Originals: SHA-256 checked, PDFKit view, "Keep offline" | `core/src/originals.rs`, `host/Documents.swift` | `4c9910e6` |
| Mac NSH trial (`SJEL_NSH_TRIAL=1`) registers `sjel.pdf` + `Originals` | `src/nsh_trial.rs` | `81cd6935` |
| NSH Mail triage screen, one screen for phone and Mac (`NshHostScreen`) | `ui/src/nsh_mail.rs`, `core/src/shell/feed_mail.rs`, `ios/src/`, `src/nsh_trial.rs` | this section's commit |

Trial result, 2026-11-07, on a paired iPhone: the list fills from the Mac, a long press on the
text opens the system menu, and VoiceOver reads the list and the reader.

### How the phone shows an NSH screen

`host/App.swift` mounts one hidden NSH view over the GPUI view at launch (`sjel_ios_nsh_mount`).
`host::show_nsh` and `host::hide_nsh` switch it. `select_tab` sets `NshPage::Documents` or
`NshPage::Mail` and shows `NshHostScreen`, and "Back" hides it (`nsh_documents::set_close`,
`nsh_mail::set_close`). Native views come from the host as NSH native components: `sjel.pdf` is a
PDFKit `PDFView` (`host/Documents.swift`, registered in `ios/src/nsh.rs`). A new NSH screen goes in
`ui/src` behind the `nsh` feature, reads through `device::Source`, and joins `NshHostScreen`.

Run it: `apps/gpui/ios/run-sim` (add `SIMCTL_CHILD_SJEL_TAB=documents` or `mail` to land on the
NSH screen), `apps/gpui/ios/run-device <udid>` on a cabled or network-paired iPhone. The Mac
window: `cargo run --release --features nsh-trial` in `apps/gpui` with `SJEL_NSH_TRIAL=1` (or
`SJEL_NSH_TRIAL=mail`).

NSH comes from git rev `893fb84db6` (`upstreams.toml`, `nsh`): the published 0.4.0 crates do not
build for this app. The rev moved off `cc38facfdd` on 2026-11-08 for the updater's zip 9 bump
(upstream `c85e3f7dc8`), because `nsh-updater -> zip 8.6.0` carried eight advisories, one CVSS
10.0, and `tools/audit` blocks on them, which failed the daily `host-patch` run.

### Open, in order

1. **Device check of everything since `a71af504`.** On a paired iPhone: Documents opens from
   Today and "Back" returns; "Original" opens a PDF at fit width with pinch zoom; "Keep offline"
   then airplane mode, and the kept document lists and opens. Not run yet.
2. **Documents done-when (§2).** A real multi-page PDF opens at fit width, a kept document opens
   with the Mac unreachable, and the phone's original matches the library's SHA-256. Built; the
   check is item 1. Built: cursor paging (500 rows a page) and #245 (side-by-side filing compare).
   Still open from §2: #246–#247.
3. **Mail on the phone as an NSH screen** (Priority 1 item 4), on the same pattern. Built in
   `ui/src/nsh_mail.rs` and `core/src/shell/feed_mail.rs` (`Source::mail_proposals`,
   `mail_detail`, `mail_action`, `mail_gmail_job`), wired through `NshHostScreen` on both iOS
   (`Tab::Mail`) and the Mac trial (`SJEL_NSH_TRIAL=mail`). Not run yet: device check on a paired
   iPhone.
4. **The new tab bar** after Mail. The bar is GPUI today (`ios/src/lib.rs`); with two NSH
   screens, decide whether the bar itself moves to NSH (`nsh_base` tabs). Not decided.
5. **Retire the phone's GPUI pages** as each is next changed. `ios/src`'s GPUI shell and
   `gpui-mobile` go when the last one has moved. Android comes from NSH's Android backend.
6. **The Mac window's content** was not looked at (`SJEL_NSH_TRIAL=1`). Whether Mac pages move to
   NSH is not decided.
7. **A smaller `ui/`.** Not designed. A page that moves to NSH drops its GPUI version in the same
   change.
