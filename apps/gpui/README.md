# Sjel GPUI

Docs Mermaid fences use [`mermaid-rs-renderer`](https://github.com/1jehuang/mermaid-rs-renderer) to produce SVG that GPUI draws natively. The dependency and adoption rationale are recorded in [`upstreams.toml`](../../upstreams.toml).

A native desktop surface for Sjel. It is intentionally outside the production Cargo workspace: it
does not change Sjel's shipping UI, capability processes, or production dependency lockfiles.

The app has twenty-three pages in eight areas. The sidebar lists the areas and ⌘1–⌘8 open them,
each at the tab it showed last, so ⌘8 returns to Packs if Packs was the last Sjel page; the header
carries the open area's tabs and switches between them without a read. **Today** (⌘1) is Home and
Today as one page: what needs you now (agent writes that wait for approval, kept-running
capabilities that do not answer — and nothing at all when nothing does), the urgent and overdue
tasks, then the day's timeline of events, tasks and spending, which steps a day with ← and →.
Everything about Sjel itself sits under **Sjel** (⌘8), **Capabilities** first: every capability
with start and stop, then the agent's recent writes. **Projects**
lists projects, tasks and notes, draws tasks as a table with due and priority edited in place, and
edits a note as blocks (`ui/src/projects.rs`, `ui/src/editor.rs`, `core/src/blocks.rs`). The
iPhone uses the same page. **Ask** streams the model's answer and
read-only tool activity into the transcript; the composer steers an active run at its next model
step. Provider API keys are managed centrally in Settings → Accounts & keys, stored in the OS
keychain, and never sent through the Ask service or conversation.

**Settings** groups appearance, connection and sync, Accounts & keys, Features, updates, and
shortcuts. Its searchable contents rail can be hidden. The Feature Manager's package, lifecycle,
and local recommendation contract lives in [the architecture guide](../../docs/architecture/feature-manager.md).
Settings can browse the official feature catalogue, inspect package requirements and data access,
and install a confirmed package when the placed runtime and release are compatible. Local Ask
signals can recommend a feature, with a visible reason and a dismiss option. Installed features
remain disabled until enabled in Capabilities. Uninstall removes receipt-owned files and retains
capability data.

**Feed** triages the comms feed of the last 30 days: New, Kept and Dismissed, with a News or
Media filter, newest day first and the best-ranked entry first within a day. The keys are the
dashboard's: j and k move, s keeps, d dismisses, u undoes, Enter or o opens the reader, ⇧O opens the
original, and Escape goes back. Decisions go through `POST /comms/feed/:id/status` only, which
records them for the ranking model. The shell exchanges the app's deployment token for comms'
own credential (`capabilities/sjel-status/src/proxy.rs`). Digests, cloud processing, the review
queue and the library stay in the dashboard for now.

Feed's **Mail** tab triages Gmail proposals (comms `/triage`): Pending, Archived, Trash, Dismissed
or All, filtered by category and grouped in the order `core/src/mail.rs` owns. x selects; d
dismisses from Sjel, w toggles Gmail's Waiting label, e archives and # trashes, and the category
buttons recategorize. Every write goes through `POST /comms/triage/bulk`. Archive, trash and the
Tax or Receipts category (which redacts the stored subject and preview) ask first; Enter confirms
and Escape cancels. o opens the thread in Gmail. Sweeps, reconciling and classification passes
stay in the dashboard for now. The list renders 200 rows at a time.

**Inbox** sorts the capture inbox (comms `/captures`): links and thoughts from `sjel capture`, the
iOS share sheet or its own field, where `#buy`, `#look`, `#go` and `#learn` set the tag. The
rules in `core/src/capture.rs` suggest where each item goes: wishlist (inventory), feed or places.
Nothing moves until r or the row's button sends it, or Apply sends every suggestion. e marks
done, d dismisses, t cycles the tag, o opens the link and n types a new capture. The rules
mirror `dashboard/src/lib/capture.ts`; change both together until the web page retires.

**Calendar** draws the calendar capability as a month grid or a week of day columns, with the
selected day's entries beside it. Arrows or h/j/k/l move the day, [ and ] page, t is today, m and
w switch the view, e steps through the day's entries, n adds one (title, time or all day, kind,
commitment) and ⌫ deletes the selected one after Enter confirms. A day window is `[from, to)`
(`capabilities/calendar/src/store.rs`); grids and the entry draft live in `core/src/calendar.rs`.
Contexts, rhythms, free days, Google import and export, and trip drafts stay in the dashboard.

**Documents** is a native local library view over the Documents capability. It filters Inbox, Filed, or All; searches filenames, extracted titles, categories and project labels; sorts by filename; and opens one record's locally extracted text and original PDF. The reader shows independent source proposals, exact evidence, disagreement, and append-only review history. Agree, Reject and Override record a review only; **File as …** remains a separate explicit filing action. Regenerate rules creates a new proposal run without replacing earlier records. Model-based classification is not wired yet.

**Travel** opens a trip library, with current and upcoming trips first and past trips behind
Show past trips. Open a trip for four subpages: **Overview**, **Itinerary**, **Map** and
**Saved items**. Overview lists the locations; Itinerary shows the saved stages and dated items;
Saved items includes activities, stays and bookings with their recorded properties and source
links. A stop's Show on map action opens Map at that location. Selecting a numbered map marker
shows its name, address and coordinates. Trips returns to the library, and each subpage opens at
the top of its scroll area.

The Mac sidebar, ⌘7 and `sjel://travel` open Travel; `sjel://map` opens its map subpage. Trip links
use `sjel://travel/<id>/overview`, `/itinerary`, `/map` or `/items`, and Copy link keeps the trip
and subpage. The iPhone's Travel tab uses the same views. `ui/src/travel.rs` owns the state, with
rendering and reads under `ui/src/travel/`; `core/src/travel/` holds detail contracts and routes.
The clients read `/trips/api/plans` and load `/trips/api/plans/<id>` when a trip is opened, using
the Mac shell or the phone's signed device client. Successful reads are saved locally with their
timestamps. Open a trip while connected once to retain its items for offline reading. Refresh
failures keep the previous answer visible. Creating plans and editing bookings use the dashboard.

The default map uses OpenStreetMap raster tiles, GPUI images and labelled stops, without a webview or
another rendering dependency. Drag to pan; the scroll wheel and a trackpad pinch zoom, both about
the pointer rather than the centre. Fit trip and the zoom buttons sit on the map. The map takes the
wheel while the pointer is over it, so the page scrolls when the pointer is anywhere else. On
desktop the stops list sits beside the map; on iPhone it sits below. Selecting a stop recenters the
map and highlights its marker. Lines connect saved stage endpoints; they are not calculated road
or rail routes. Only the visible Map subpage requests tiles, four at a time, and any request for a
camera the gesture has already left is cancelled. Attribution stays visible. Viewed tiles are
cached on disk; **Cached only** prevents new tile requests. Loading and map errors appear over the
map, and missing areas offer retry.

**Imagery is never thrown away mid-gesture.** The camera is a fractional zoom, and both map paths
draw what is already on screen with the gesture applied: a tile of a coarser level is drawn at
`256 · 2^(zoom − tile.zoom)`, and a tile that is missing is covered by its nearest cached ancestor
upscaled, so a zoom or a pan never reveals a blank square. Requests go out once the gesture has been
quiet for 140 ms (`MAP_SETTLE` in `ui/src/travel/map.rs`). Kept tiles are bounded at four levels
around the current one, 400 tiles in all. The loading chip is reserved for "nothing to look at
yet"; it does not blink over imagery that is already there.

This cache covers areas already viewed, not a downloaded city or country. OpenStreetMap's
[standard tile policy](https://operations.osmfoundation.org/policies/tiles/) prohibits bulk and
offline prefetch, so downloadable map regions need a provider that explicitly permits them.
The phone needs the Mac once to obtain trips, but loads map tiles directly over its own connection.
Opening Map loads its visible area automatically.

**A typed place gets its coordinate here, and only here.** Trips stores a place as it was typed:
the dashboard's place field mints `place:<slug>` with a null coordinate unless the operator picks
a suggested station, and `trips draft-intent` does the same on purpose. Both maps downstream then
had nothing to draw — the world overview in this app, and no pin in the dashboard. `core/src/travel/places.rs`
resolves those names through `POST /places/api/geocode` (the capability that owns geocoding,
started on demand) and keeps the answers in `places.json` beside the plan snapshot. A coordinate
that arrives with the record is never overridden. The cache is why the map is instant on a second
open and still correct offline; the dashboard reads the stored record and stays unresolved until
the capability resolves it server-side. Places the geocoder cannot place keep reading
"Coordinates not available" — the map falls back to its world overview and the stops that do
resolve still draw. While the first answer is in flight the map holds back its world-overview tile
request rather than spending it on twelve tiles the fitted view throws away.

### Native vector map prototype (macOS)

Build and install the experimental MapLibre Native path with:

```sh
apps/gpui/install --native-maps
```

Travel → Map starts with OpenFreeMap's vector basemap. **Use raster map** switches back to the
existing implementation. This requires no account, API key or webview. The feature flag is
`native-maps`; the installer uses the binding's pinned precompiled native core and includes its
license notices. The installed app inherits `SJEL_GPUI_NATIVE_MAPS=1`, so its own Rebuild action
keeps the prototype enabled. `SJEL_GPUI_NATIVE_MAPS=0 apps/gpui/install` restores the default build.

`ui/src/travel/native_renderer.rs` owns the native engine; `vector.rs` owns a single worker and
one pending request. MapLibre objects stay on that thread. Camera changes render an image,
convert RGBA to GPUI's BGRA format and compose it beneath the GPUI itinerary markers. The
camera accounts for MapLibre's 512px tile convention versus the raster view's 256px convention.
Stale camera results are discarded.

Every wait goes through the binding's own `StyleLoadRequest::wait` / `RenderRequest::wait`. Those
park on the thread's MapLibre run loop, which on macOS is CoreFoundation: a completion callback
only runs while that loop runs, so a hand-rolled `RunLoopHandle::tick()` + sleep loop never
delivers one and each attempt spends its whole budget before failing. Measured here: the same
Berlin frame renders in 837 ms through `wait()` and never completes through tick polling — which
is what used to make opening Map stall for 12 s and then fall back to raster.

The renderer is built with a pixel ratio equal to the window's scale factor, so a Retina window
gets one image pixel per device pixel at the same geographic extent; the frame is drawn at point
size. The worker rebuilds its map when that ratio changes. The wait itself carries no deadline, so
`vector.rs` caps the caller side at 12 seconds and switches to raster when it expires.

Between renders the frame follows the gesture: it is drawn scaled by `2^(zoom − frame.zoom)` and
centred on where the frame's own centre projects at the current camera, so a pinch or a pan answers
immediately and the crisp frame arrives one settle later. That is why the vector path does not need
MapLibre's continuous renderer: every frame of that path is a full Metal render plus a CPU readback
of the whole image, paid sixty times a second to show a map that is redrawn once per gesture.

This prototype copies pixels rather than sharing Metal textures. It does not yet establish
smooth continuous rendering, iPhone support or downloadable offline regions.
Vector mode requires a network connection; the raster cache controls apply only to raster mode.
The default iPhone build still uses the shared raster view. The adoption verdict, exact pin and
licenses are recorded under `maplibre-native-rs` in the root `upstreams.toml`.

A standalone render probe writes a public Berlin map without opening a window or reading trips:

```sh
MLN_PRECOMPILE=1 cargo run --manifest-path apps/gpui/Cargo.toml -p sjel-gpui-ui \
  --example native-map --features native-maps --locked -- /tmp/sjel-maplibre-berlin.png
```

The process needs access to a Metal device. A sandbox without GPU access cannot validate the
render, even when compilation succeeds; the adapter reports that condition before entering
native core.

Every block keeps its markdown verbatim, so a save writes back exactly what the editor did not
change; a note that does not round-trip opens read-only. A save follows each pause in typing and
sends the content it started from, so the capability refuses an edit made over a newer version.

The app reads the shell (`sjel-status`) over loopback with the inbound token, through the same
endpoints as the menu-bar app (`core/src/shell.rs`). The token comes from the login Keychain item
`sjel-inbound-token` that `tools/setup-inbound-auth.sh` writes, read through `/usr/bin/security`,
which that script puts on the item's access list. The token stays in the app process and never
reaches the browser or the log.

`GET /api/sjel-status/events` pushes the health and agent answers when they change
(`capabilities/sjel-status/src/status/events.rs`), so the open window reads them once and then
listens: one held connection and one calendar read per 10 seconds, instead of three reads. A shell
without that stream keeps the timer.

A project's records arrive without their content, and the note editor reads the one record it
opens. A task's done, due and priority go to the capability, which owns the frontmatter edit and
answers with the record's new frontmatter, so the table patches one row instead of reading the
project again. `core/src/data/` holds every read and write, and `ui/src/data.rs` keeps the half
that needs a window: a read settles into the slot the page draws and never runs twice at once, and
a write toasts its outcome and then lets the caller re-read only what it changed.

Where a local `axon.db` is readable, `Repo` in `core/src/data/repo.rs` answers projects, tasks,
calendar and record reads from it and falls back to the shell otherwise, over one LibSQL handle
opened once per process and shared by every clone. Holding the connection rather than reopening it
is the point: opening one costs 0.05 ms, but a fresh connection starts on an empty SQLite page
cache, so opening and reading once costs 0.68 ms against 0.02 ms for the same read on a handle that
already ran one, and a poll runs several. How the numbers were measured: `cargo test --release
--manifest-path apps/gpui/core/Cargo.toml holding_one_connection -- --ignored --nocapture`, which
times all three against this machine's database.

Going back to a note opened in the last five minutes costs no read at all. `core/src/data/notes.rs`
keeps the 16 notes opened most recently, most recent first, so tabbing back to one is a memory hit
and the frame the click lands in: 0.42 µs at p50 for a 21 KB note, in a release build. Every write
the app makes to a record drops that record, so a kept note is never older than the last write, and
the editor leaves the body it saved in place of the one it opened.

The phone reads a different file for a different reason. Its Projects page runs over
`Repo::replica`, which reads the phone's replica and never asks a shell. Every write lands in the
replica and in `_offline_mutations` first, so the page behaves the same whether or not the Mac
answers, and `core/src/sync.rs` replays that queue later. A project made on the phone is its hub
note until the sync creates the project on the Mac. `Board` in `core/src/data/board.rs` keeps the
count of writes still queued, for the badge and the sync. The Mac app keeps no replica of its own.
It reads the host's database through `Repo`, so `Board::default_path` names a file only in the
phone's app container, and `SJEL_REPLICA_PATH` is how the Mac tries the phone's view.

The app and the web dashboard share one set of destinations. `core/src/route.rs` copies the paths
and labels of `dashboard/src/lib/nav.ts`, and a test fails when the two differ. A page with a
native view opens here. Every other destination opens the dashboard in the browser, logged in
through a single-use ticket. `sjel://<path>` links follow the same rule: `sjel://capabilities`
opens the native page, `sjel://calendar` opens the dashboard at `/calendar`.

## Why this shape: GPUI for every client

GPUI is the goal for every Sjel client, on the Mac and on iOS. AppKit or UIKit draws the
operating-system chrome, and GPUI draws the content. The workspace pins one GPUI snapshot,
`gpui-pre =0.3.9` with gpui-kit 0.7.3, and uses the Longbridge `gpui-pre-mobile` fork on iOS.
Sources: `upstreams.toml`, the mobile page of gpui-kit's website docs, and `src/macos_glass.rs`.

It is a stack decision, not a speed fix: Sjel's waits come from services, not from rendering. A
release build starts in one frame under 10 ms with about 100 MB resident — against about 220–300 MB
for a Tauri and WKWebView process pair — and the Swift menu bar keeps system-wide ⌃⌥Space, so global
invocation pays neither a second resident process nor a cold-launch hop. How the numbers were
measured: `SJEL_GPUI_FRAMES` and `ps -o rss=` in `src/frames.rs`.

The move has four steps:

0. **Done.** One Cargo workspace builds GPUI, gpui-kit and gpui-mobile, and shows one list fed over
   loopback HTTP on the Mac and in the iOS Simulator. Had this failed, the iPhone decision would
   open again.
1. **Done.** A command palette, measured for start time and resident memory against Tauri.
2. **In progress.** The dashboard's routes, ported into this app: `context`, `calendar` and `inbox`
   first, with the native `capabilities` page as the reference. GPUI draws the charts. Travel now
   has a shared native map view: raster maps work on both clients, with the MapLibre vector
   prototype available on macOS. The dashboard keeps its vector map. Home, Today, Capabilities,
   Systems, Projects, Ask, Travel, Feed, Inbox, Calendar, Documents, Backup, Finance, People,
   Inventory, Software and Import are native. `/systems` is the one that is not a capability's
   page: it puts macmon's live sample beside storage, updates, the runtime profile, the hourly
   host watch and the agent's reach, each read from whoever already owns that answer (`src/systems.rs`).
3. Test and review, then decide: switch, keep both, or stop. Svelte and Tauri stay live until that
   review.

## Workspace and the iOS app

This directory is one Cargo workspace with one lockfile, so every member builds against the same
GPUI snapshot (step 0 above).

| Member | What it holds |
|---|---|
| `.` | The Mac app |
| `core/` | What both apps share: the destinations, the shell client, a paired device's client, the offline replica and the two-way sync. No GPUI types. |
| `ui/` | What both apps share for GPUI: semantic design tokens (Palette), density rules, layout primitives, document/block renderer, the Projects page, the note editor and navigation bar. |
| `ios/` | The iOS app: GPUI content in `src/lib.rs`, the UIKit host in `host/` |

On iOS, UIKit owns the window, the navigation bar and the safe areas. GPUI draws the content in a
child view controller. `gpui-pre-mobile` comes from git rev `c24a1308e1`, because its only release
(0.1.0) pins an older GPUI. That rev also pins `libc =0.2.189`, so the whole workspace uses that
version. The iOS 27 SDK starts only apps with the UIScene life cycle, so the host uses a scene
delegate.

The iOS app has no inbound token: it is a paired device (`core/src/device.rs`), signed with its
Keychain key over TLS pinned to the Mac's LAN listener. The Mac is found over Bonjour:
`host/Discovery.swift` browses `_sjel._tcp`, whose TXT record carries host, port and certificate
fingerprint, and hands every result to `sjel_ios_nodes` in `ios/src/lib.rs`, which shows the card
the person taps Join on. `SJEL_JOIN` and `SJEL_PAIR` stay as the fallbacks for a host that cannot
browse and for the QR code. What does not exist yet is the camera scanner that would read that QR
code on a phone.

`ios/run-sim` builds the app, runs it on a headless simulator and prints a screenshot path; it
needs Xcode, XcodeGen and the `aarch64-apple-ios-sim` Rust target, and never opens Simulator.app.
`ios/run-sim --tap` runs the touch check instead: taps Join on the discovered Mac and keeps a
screenshot before and after, because GPUI paints its own pixels and a screenshot alone cannot show
whether a finger reached it.

The iOS app reads and writes a local replica (`core/src/store.rs`, LibSQL in the app container), so
the store, the tasks and the notes answer with the Mac out of range. Every local write is also
logged in `_offline_mutations`, and `core/src/sync.rs` replays that log at the Mac once the pairing
is live: the queued writes go out oldest first, then every live project is listed without its
contents and only the records whose `updated_at` moved are read in full. A record with an unsent
local edit is skipped by the pull, and a record the Mac no longer has is deleted here. Two edits of
the same note are settled in the phone's favour: the queued text is replayed over the Mac's version
on the 409 and the report counts it, so nothing typed offline is lost. A note made on the phone in
a project the Mac does not have yet creates that project first. The sync runs when the pairing
answers at launch, after every local write, and on the Sync button; a phone that has not reached
the Mac this launch never starts one, because its queue is the only copy of those edits.

The iOS app works with no Mac at all (`research/offline-on-the-phone.md`, "Decided 2026-10-22: the
phone stands on its own"). It opens on **Today** (`core/src/today.rs`, `ui/src/today.rs`): the day
the Mac's copy answered, then the tasks the phone's own replica holds with a checkbox the thumb can
hit, then the projects those tasks are filed under, each with its progress and a row that opens it in
the Mac's Projects page. A page that reads two sources says so: one line names the copy and its age,
one says what is queued and whether this launch is connected. Nothing on it is a chart or a greeting.
The bar holds four items at its 64 px floor, so Projects is not one of them — it is the list a
project on Today opens — and **Settings**, which holds the pairing: the found Mac, the Join tap, and
the capability health list it answers with once the owner allows the device, sits behind Today's
gear, one row from it. **Projects** is where a person writes: the Mac's Projects page over the
phone's own records, under the queued/synced badge and the report of the last sync. Tables drop the project and
type columns at phone width. The software keyboard opens with a note and after a tap into a text
field, because gpui-mobile raises none on focus. A fresh install starts empty and queues
nothing, so pairing later sends the Mac only what the person wrote. A store that cannot open says
so on that page instead of falling back to memory.

**The right edge** is one panel with seats, not a drawer per feature (`ui/src/sidebar.rs`): the frame
draws the title, the seat chips and the close button, and a seat is the content a host hands in. On the
phone it is an overlay 340 pt wide from the right edge with the page dimmed beside it — a phone has no
width to dock beside a page — and the same frame docks where a Mac has the room. Which seat is showing
and whether it is showing at all live in `sidebar.json`; the frame's scrim is `.occlude()`d, so a tap
that closes the edge cannot reach the page underneath it.

**Soundscape** is the first seat: the note in Today's header opens the edge on it, and it holds the
transport, the six presets, the volume and a mixer of five layers (`ui/src/soundscape.rs`). The engine
is the Mac's own (`core/src/soundscape`, platform-free); the difference is who drives the clock. On the
Mac a CPAL stream asks for frames; on the phone `AVAudioSourceNode` does (`ios/host/Audio.swift`,
`ios/src/audio.rs`), on a `.playback` session that mixes with whatever else is playing. The seat keeps
its mixer disclosure, its preset and its volume in `soundscape.json` beside `appearance.json`; it does
not keep `playing`, so a phone never starts making sound by itself. A value is a row of ten cells rather
than a track to drag: a thumb wants an exact tap, and a measured rectangle is one more thing to get
wrong.

**Activity** is the second seat, behind the list button in Today's header: what this phone is doing, in
its own words — the Mac's answer, the last sync, the queue, and the reminders the notification centre
holds. It is where an agent's steps will stream once a phone client has an Ask turn to stream, which is
the plan's own point about Sjel working in view (`docs/plans/gpui.md` §4).

**Local alerts** use the phone's own notification centre: no push registration, no device token and no
server that would need one (`core/src/notify.rs`, `core/src/travel.rs`, `ios/src/notify.rs`,
`ios/host/Notifications.swift`). The card at the top of Settings carries one switch for calendar and
task reminders and low-fare alerts, the lead time (`At start`, `10 min`, `1 hour`) and
the notification centre's own answer — the count and the next moment come from
`getPendingNotificationRequests` and `nextTriggerDate()`, so the line cannot claim more than iOS holds.
What earns a reminder: a timed entry at its lead, an all-day entry at 08:00 on the day it covers, an
open task at 09:00 on its due date; an entry whose commitment is `possible` blocks nothing and earns
nothing. A new `sparpreis-low:` note in an upcoming trip is announced once when the app opens or
returns to the foreground; the note ID is kept on-device, and saved trip details let the check run
without the Mac. The permission is asked when the switch is turned on and never at launch, because iOS
offers that prompt once; a refusal says where to change it and offers the way there. The rule is rebuilt when
the replica opens, after a write, after a pull, and when the scene becomes active — a phone in a pocket
crosses days without the app running. The switch and the lead live in `notifications.json`; what is
scheduled is never read from a file. A tap opens the day it announced on the Calendar tab.
`SJEL_GPUI_NO_NOTIFICATIONS=1` keeps the app away from the notification centre entirely, for a run that
has no business asking for a permission.

**Inbox** is the next tab: one field for whatever is in hand, four chips for the tags the comms
capability knows, a line saying where the capture will land, and `Keep`. It writes the phone's own
replica first and queues the capture for `POST /comms/captures`, so it works on a train; the list
under the field mixes this phone's captures with the Mac's — one `GET /comms/captures` per sync,
matched by the Mac's id so a capture typed here and then landed is one row and not two — and says
which have landed (`On the Mac`), which are waiting, which the operator has already dealt with
(`Done on the Mac`, `Dismissed on the Mac`), and, when the Mac says no, its reason on that row. Triage is not here — a wishlist item, a feed source or
a place is the Mac's decision (`core/src/capture.rs` holds the rule both sides read).

**Calendar** is the next tab, and it reads the same replica as Projects once the Mac's change feed
has been read: `GET /calendar/api/entries/changes?since=` answers what moved, and only the cursor
keeps the pull cheap. Until that first pull lands — a fresh install, or a phone that has not reached
the Mac yet — the tab draws the Mac's read-only mirror instead (`core/src/mirror.rs`,
`ui/src/calendar.rs`), which is the route R1 copy `sjel mirror build` writes and
`GET /api/sjel-status/mirror` serves: the manifest travels first, so a copy already current costs one
small answer; the bytes are checked against the manifest's digest before they replace anything, and
the new file is renamed into place, so a fetch that dies halfway leaves the previous copy whole. The
line under the day's title says which source drew it, and only a day off the replica carries an
editor. **New** writes an entry on the open day into the replica — title, all-day or 09:00–10:00 and
a commitment; a row's **⋯** asks and a second tap deletes — and the queue carries both to the Mac on
the next sync, where a phone-minted id is replaced by the one the Mac assigns. Both writes are absent
over the copy, because a copy takes no writes. **Refresh** syncs when there is a store, and otherwise
asks the Mac to rebuild and refetches the copy. `places_*` and `entities_*` are already in the same
copy, so People and Places are a page away; the pages that write need a domain of their own, which is
the shape the calendar just took.

**People** is the last tab but one: the contact registry and the places register, one behind the other under
a `People` | `Places` switch with one search field for both, read from the same copy the calendar
reads — one fetch feeds both tabs. A person's row carries their initials and what they do; the card
holds the email, the phone, whatever else the registry has on them, and the places the register links
to their name. A place's card holds its kind and where it is, and the names linked to it, `confirmed`
and `proposed` apart. The tab is read-only by intent: `places_person_places` is the operator's
review, and the registry's writes are `/api/people/decide`, so a phone that cannot reach the Mac
offers no control that would have to be refused.

Two things this tab taught about the copy and the registry. The phone set did not carry
`entities_values` (phones, emails, birthday, last_contact, company, relation) or `places_people` (the
operator's decision for a person's name), so a page over it had 294 names and no contact card; both
are in `PHONE_SET` now, and the copy is 2,493 rows in 11.3 MB instead of 1,878 in 11.1 MB. And
`Person::from_record` looked for `email` and `phone` while `capabilities/entities` writes `emails`
and `phones` as arrays, so no contact card on either client had ever shown an email or a phone
number; the parser accepts both spellings now, which is a Mac fix as much as a phone one.

The tabs ship on TestFlight (People arrived in build 1363, internal group `Team`), the only
distribution today. The touch checks are `host/UITests/`: `JoinTap.swift`, `ScanTap.swift`,
`TodaySwipe.swift` (which drags the landing page up and proves both that a finger reaches GPUI's
scroll and that the projects below the fold are really drawn), `InboxCapture.swift`,
`SoundscapeTap.swift`, `NotificationsTap.swift` and `SidebarTap.swift`. `SJEL_TAB=today|projects|calendar|people|travel|settings`,
`SJEL_REGISTER=places`, `SJEL_PERSON=Ada`, `SJEL_DAY=2026-10-21`, `SJEL_NOTIFY=1` and
`SJEL_SIDEBAR=soundscape|activity|off` open a given tab, register, card, day, the reminder card with its
switch on, or the right edge on a seat (or away), at launch — which is how `ios/run-sim` and a
screenshot reach a page that is not the first one without a hand on the screen. Only the scanner check
asserts anything: GPUI paints its own pixels, so the rest are screenshots a person reads.

A write the Mac will not take is named rather than counted. `sync` used to stop at the first refusal,
so one duplicate note title held every later write in the queue and nothing said which record it was
(`#244`). Now a refusal blocks its own record and the queue keeps draining behind it, the reason is
written into the replica (`_sync_refusals`), and the Projects page draws a band above the list: the
record's title and kind, the Mac's reason in its own words, "Saved here and still queued. Nothing was
lost or renamed.", an `Open` per record and one `Try again`. A record is never renamed or deleted to
make a refusal go away — a duplicate title and an archived project are the operator's decision, and
`Try again` on a phone with no Mac says so instead of pressing a dead button.

The project a task is filed under came back to the phone in this pass, and it is not the path's first
folder: the vault files `Archive/Soma/Tasks/Reopen.md` under `Soma`. `ProjectRecord` had dropped the
capability's own `project` field, so `Board`, the sync's pull, and the landing page's grouping would
all have said `Archive` — 22 of the copy's 62 task rows, measured 2026-10-24. The field
travels now (`core/src/shell.rs`), the replica queries read the column, and `filed_under` in
`core/src/sync.rs` prefers it.

### Shipping it to TestFlight

One command archives, signs, uploads, waits for App Store Connect to process the build, and sets
its What to Test note from the latest commit subjects:

```sh
sjel testflight            # or: sjel testflight --notes "…"
```

The record is `Sjel Life` (the store name `Sjel` is taken), the bundle `dev.sjel.ios`, the SKU
`sjel-ios-001`. The icon is `host/Assets.xcassets` (rebuild it with `host/make-icon`), and
`host/PrivacyInfo.xcprivacy` declares the two required-reason APIs a Rust binary actually touches.
`ITSAppUsesNonExemptEncryption` is false, because the app encrypts nothing beyond TLS. What the
App Store Connect key can and cannot do, and the one-time steps in the web UI, are in
[the operations guide](../../docs/operations/index.md).

## Soundscape: a slideover with its own synth

`⌘0`, `⌘⇧M`, the sidebar's ♪ row, or **Window ▸ Soundscape** slides a 400-point panel in from the right
edge of the window: a drawn field, the transport, the six presets, volume and energy as full-width
bars, the twelve-value mixer behind one `mix` disclosure, and the timed scenarios. The same
keystroke, `esc`, the `×`, or the same row in the sidebar slides it away again, and the music keeps
playing -- the audio device lives in an app global, not in the panel.

It is a panel and not a destination, because a soundscape is something you leave on beside a note.
It is a panel and not a window because a second window takes focus from that note, has to be found
in the window list, and its size was never the size of the thing it held: the player was 980 by 700
of mostly empty field, and between that and nothing there was no third option. The panel moves over
the page column and never over the navigation, so the way out of a page stays reachable while the
music is showing.

### Why there is a synthesizer in this app

The dashboard makes this sound in the browser, and `capabilities/soundscape/README.md` explains why
that split is right for a page: the conductor owns *what* should be playing, the browser owns
oscillators and the clock. A native app has an audio device, so the same contract lands differently
-- the conductor still owns intent (preset, parameters, layers, seed, session), and
`core/src/soundscape/` executes it against the device's own sample clock.

- `core/src/soundscape/music.rs` is the browser's musical material, moved to Rust: the chord tables,
  the EDM motif, the scape scales and roots, the six drift LFOs, the arrangement arc. Two surfaces
  that sound different while both claiming to play "focus" would be worse than one surface.
- `synth.rs`, `fx.rs` and `engine.rs` are not a port: voices are fixed slots in an array with a
  closed-form envelope instead of scheduled Web Audio nodes, the room is Freeverb rather than a
  convolution of generated noise (a six-second impulse is a twelve-second FFT), and there is no
  lookahead scheduler because the audio callback *is* the clock. Each of those files says so where
  it deviates.
- `src/audio.rs` is the Mac's device, through `cpal`. The stream lives in a `Global`, not in the
  view, so closing the panel does not cut the music.
- `src/viz.rs` draws the field from the synth's own spectrum: seven ridges, one per slice of the
  bands, drifting at their own rates, with a ring per kick and a comet per note. It is not the
  dashboard's fluid sim -- that one is a GPU solver in a browser canvas -- and it is not a
  simulation at all: every number in it comes from what is sounding.

The conductor stays the only source of truth. Every control writes a *patch* (`POST
/soundscape/api/soundscape/state`), never a whole state, because the dashboard and the phone drive
the same session; the panel polls the conductor every three seconds so another surface's change
arrives here on its own. When the conductor is unreachable the panel keeps sounding the last known
intent and says so in the header -- it writes nothing, and it never becomes a second store.

| Key | Action |
|---|---|
| `space` | Play or pause |
| `⇧⌘N` | Next preset |
| `m` | Open or fold the mixer |
| `⌘0` or `⌘⇧M` | Slide the panel in, or away |
| `esc` | Slide it away; the music keeps playing |

A bar is its own control: press anywhere on the track to set the value, drag to sweep it. A dragged
value reaches the synth immediately and the conductor at most every 180 ms, so the sound never lags
the hand and a sweep is one write rather than dozens.

The sidebar's ♪ row, the finder and the global keys (`⌥⌘Space`, `⌥⌘P`) write play and preset
through the panel, so the panel is the one writer of both and its guard against a stale poll covers
every surface.

Each preset carries its own level, echo and air (`Preset::trim`, `Preset::echo`, `Preset::air` in
`core/src/soundscape/music.rs`), so a preset change is not a volume change and Ambient's repeat is
longer and quieter than EDM's. `cargo test -p sjel-gpui-core --lib mix_report -- --ignored
--nocapture` prints each preset's RMS, peak and band profile; it is the measurement any change to the
mix starts from. A stolen voice fades over 3 ms instead of stopping, pads drift slowly in the field,
and melodic notes land up to 6 ms late and ±8 % in level on their own random stream, so a seed still
picks the dashboard's notes.

`SJEL_GPUI_AUDIO_PROBE=<seconds>` is the check for "there is no sound": it starts the synth on the
real device, prints the measured level, spectrum and event counts for that many seconds, and quits
with a verdict. `SJEL_GPUI_SOUNDSCAPE=1` opens the panel at launch and `=play` also starts it
through the conductor, which is what a screenshot test or a quick listen needs.
`SJEL_GPUI_NO_AUDIO=1` keeps the app off the output device -- for a headless run, or a test that
opens the panel without taking the speakers.

## The finder: one input for a search or a question

⌘K opens one input over the window. It searches the pages, the app's actions, the dashboard
destinations, and the projects, open tasks and notes in the Mac's database, and it asks. In
**Auto** the words decide: a question mark, an opening question or request word ("what",
"summarize", "wann"), or five words and more make a question; a name or two keywords make a
search (`core/src/search.rs`). The decision shows beside the input, ⌘↩ goes the other way, and ⌘/
switches between Auto, Ask and Search. An arrow key picks a result, and Enter opens a picked result
whatever the words look like. In Auto, a search that finds nothing is asked instead.

A question is answered in place, and Ask has no page of its own. The conversation stays above
the results while the input is empty or holds a follow-up question. Typing a search hides it, and
*new conversation* clears it. The footer shows where the next question goes: *local model*, or a
cloud role. A cloud role receives the current prompt only, after *mark public c0* is clicked for
that send. Provider keys are in Settings, under *Accounts & keys*. ⇧⌘A, `g a` and `sjel://ask`
open the finder too; `sjel://ask?q=…` also asks.

Local results show on every keystroke. When typing pauses for 180 ms, the shell's
`GET /api/sjel-status/search` adds what the app does not hold: calendar events, people, documents,
inventory items, subscriptions, trips and mail subjects, five per source, below the local results.
Each server hit carries a `sjel://` link and opens through the deep-link parser. When nothing
matches, the finder says what Enter will do instead.

## Keys

| Key | Action |
|---|---|
| ⌘K | The finder: search, or ask (see below) |
| ⌘1 … ⌘8 | Today, Inbox, Calendar, Projects, Documents, Travel, Life, Sjel — each at its last tab |
| ⌘⌥↑ / ⌘⌥↓ | The area above or below in the sidebar |
| ⌘⇧[ / ⌘⇧] (also ⌃⇥, ⌘⌥←/→) | The tab before or after the open area's |
| `g` + a letter | A page by chord: `g x` Systems, `g k` Packs, `g y` CV, `g ,` Settings |
| ⌘R | Refresh |
| ⌘D | Open the current page in the dashboard |
| ⌘0 or ⌘⇧M | Slide the Soundscape panel in or away |
| J / K, ↓ / ↑ | Select the next or previous row |
| ↩ | Run the selected row's main action: Allow, Start, or open the calendar in the dashboard |
| ⌘⌫ | Run the selected row's costly action: Deny or Stop |
| Esc | Clear the selection |

A selected row shows the actions that a mouse sees on hover, for example Stop.

In a note, ↩ starts a block and ⇧↩ a new line; ⌫ at a block's start turns a list item or heading into
a paragraph, then joins it to the block above; `# `, `- `, `[] `, `1. ` or `> ` typed at a
paragraph's start changes its kind. `⌘S` saves, `⌘[` goes back to the project, and `/` in an empty
paragraph opens the block menu (also on the `⋮⋮` handle, which also drags). Only the block with the
caret shows its markdown; every other block shows its formatting with the markers hidden
(`core/src/inline.rs`).

A project page shows its tasks as a table above its notes (`src/tasks.rs`): open first, then due
date, then priority. A click on the due date edits it (↩ saves, leaving keeps the old date), a click
on the priority cycles P1, P2, P3, and the last row adds a task. `SJEL_GPUI_NO_SAVE` covers the note
editor only; the table always writes.

## Running it

The default endpoint comes from `capabilities/sjel-status/service.toml`. `SJEL_STATUS_URL` can point
to another loopback port. The app refuses any endpoint that is not `http` on 127.0.0.1 or
localhost.

```sh
cd apps/gpui
cargo run          # development
./install          # ~/Applications/Sjel Desktop.app, opened by the menu bar's Open App
```

Both build into `CARGO_TARGET_DIR`; `./install` falls back to `$HOME/.cargo-target` when that is
unset, so the in-app Rebuild shares the terminal's warm cache instead of building its own.

`./install` also registers the `sjel://` scheme. Run it again after a change.

## Testing without taking focus

When a sandbox blocks PID inspection, Interceptor can report a live daemon as stopped or owned
by another user. Confirm its loopback `/health` response and process ownership before changing
the installation. Use `interceptor --ws` for browser and native commands against the running
daemon; this transport avoids the CLI's process probe. `preview` selects it explicitly. A
successful `interceptor --ws macos trust` and a background screenshot verify native access;
`interceptor --ws contexts --verbose` verifies the browser extension. Do not reset permissions
or reinstall solely because the PID probe failed.

These checks run while you keep working in other apps. None of them activates the app.

| Command | What it checks |
|---|---|
| `cargo test --workspace` | Routes, calendar rules, the shell client and its stream parser, keyboard navigation in a headless GPUI window, and the synth (every preset, eight bars, envelopes, the room, the spectrum) |
| `./preview [page] [light\|dark]` | Screenshot of the installed app, through Interceptor |
| `./bench-scroll [page]` | CPU cost of one scroll frame, from the app's own scroll run |
| `SJEL_GPUI_BENCH_NOTE=<record path> ./bench-scroll projects` | CPU cost of one keystroke in that note; saving is off |
| `ios/run-sim` | Screenshot of the iOS app on a headless simulator |
| `ios/run-sim --tap` | The touch check: taps Join on the discovered Mac, screenshots before and after |

`preview` and `bench-scroll` show the window for a few seconds in the bottom-right corner, above
other apps. GPUI paints nothing while another window covers it, so a hidden window cannot be
measured. Run `./install` first: both scripts use the installed release build.

| Variable | Effect |
|---|---|
| `SJEL_GPUI_NO_ACTIVATE=1` | Start and follow `sjel://` links without taking focus |
| `SJEL_GPUI_FLOAT=1` | Open the window in the bottom-right corner, above other apps |
| `SJEL_GPUI_OPEN=sjel://…` | Open this destination after start |
| `SJEL_GPUI_KEYS='wait 1000; type Köln Hbf; keys enter'` | Replay waits, keystrokes and typed text into the window after start (`src/script.rs`) |
| `SJEL_GPUI_APPEARANCE=light\|dark` | Preview a palette without changing the system setting or the mode saved in Settings (⌘,) |
| `SJEL_GPUI_OPAQUE=1` | Draw an opaque window instead of the blurred one |
| `SJEL_GPUI_NO_GLASS=1` | Draw the normal sidebar instead of the glass one |
| `SJEL_GPUI_FRAMES=<file>` | Log the timings of each frame (`src/frames.rs`) |
| `SJEL_GPUI_BENCH_SCROLL=1` | Scroll the page down and up, then quit |
| `SJEL_GPUI_NO_SAVE=1` | Edit notes without writing them, to try the editor on real notes |
| `SJEL_GPUI_BENCH_NOTE=<record path>` | Open that note with saving off, type 240 characters, then quit |
| `SJEL_GPUI_SOUNDSCAPE=1\|play` | Open the Soundscape panel at launch, and with `play` start it through the conductor |
| `SJEL_GPUI_AUDIO_PROBE=<seconds>` | Run the synth on the real device, print what it measures, and quit |
| `SJEL_GPUI_NO_AUDIO=1` | Keep the app off the output device, for a headless run or a panel test |

The app icon comes from `brand/sjel-icon-maskable.svg`. `Resources/make-icon` builds
`Resources/AppIcon.icns` from it. Run it again after a change to the brand mark.

The app uses the components of the pinned `gpui-kit = 0.7.3` facade (input, dialog,
notification, button, tag, kbd) and adds no second GPUI dependency. Its lockfile resolves one
`gpui-pre` family. GPUI is still pre-1.0, so this is not a production dependency decision.

GPUI's blurred window is not Apple's Liquid Glass: the public API has no general AppKit child-view
host, so one narrow bridge (`src/macos_glass.rs`) puts an `NSGlassEffectView` and a tint strip
behind GPUI's view and leaves the sidebar unpainted. In light mode a faint seam shows where the
strip meets GPUI's tint, because GPUI's translucent colours blend lighter over an AppKit layer than
over its own. `SJEL_GPUI_NO_GLASS=1` draws the normal sidebar on any macOS. This is an experiment,
not a supported GPUI hosting API.

## Evaluation scope

- Does a GPU-native Rust shell feel like a better desktop home for Sjel?
- Is the build and macOS packaging burden acceptable beside Tauri, iOS, and the Swift menu-bar
  companion?
- Can richer views use the existing HTTP contracts without adding direct database access or
  exposing credentials to the client?

The app is an evaluation surface, not a dashboard port. A destination gets a native page when the
native form does something the browser cannot do as well. Until then, the finder opens it in the
dashboard.
