# ISA archive · Sjel

The features of the root [`ISA.md`](ISA.md) whose every claim is ticked, with their test rows
and the log up to 2026-10-08. Nothing here is open work. A claim that is reopened moves back to
`ISA.md` with its feature. Text moved here unchanged, except two marked edits: the ISC-65 entry
of 2026-10-08, and the test commands that named `sjel-interior` for code that moved to
`sjel-inventory` (F13).

## Features

### F0 · The tracker stops being the backlog

Why: two systems of record is the actual defect; closing the issues without moving the
doctrine and the automation would refill the tracker by Monday.

- [x] ISC-1 — `gh issue list --state open` returns nothing in either repo. Evidence: 0 and 0, 2026-08-20. Falsifier:
  any open issue in this repository or the private overlay.
- [x] ISC-2 — every closed issue's live content stands as a claim or a Not-yet-specified
  entry in an ISA, and its closing comment names the file. Falsifier: a closing comment
  with no destination, or a destination that does not contain the content.
- [x] ISC-3 — the doctrine agrees with itself: `README.md`, `CONTRIBUTING.md`,
  `AGENTS.md` and `tools/doctor.ts` all name ISAs as the backlog. Falsifier:
  `rg -i "backlog is Issues|GitHub Issue"` still describes the backlog anywhere in those
  four files.
- [x] ISC-4 — `upstream-watch` opens no issue: it writes the drift report to the job
  summary and exits non-zero when an entry is past its cooldown. Falsifier: the workflow
  file still calls `gh issue create`, or a green run hides real drift.
- [x] ISC-5 — `bazel test //...` green on the landing commit, `ARCHITECTURE.md` included. Evidence: 68/68 pass, plus `//:architecture_up_to_date_test` forced uncached, 4050fe4.
  Falsifier: any failing target.

### F1 · Demo seeding for Comms, Scouting and Transit

Why: three capabilities are missing from the published demo, so Feed, Scout and Travel
are hidden and the shell looks thinner than the system is.

- [x] ISC-6 — the demo shows Comms, Scouting and Transit, every recorded value having come
  from a real server answering a real request, not a hand-written fixture. Evidence, one
  full `tools/demo-up all` on 2026-08-20: `seeded comms: 7 items ingested from the demo
  origin (3 kept, 1 dismissed)` · `seeded scouting: 7 opportunities discovered through the
  rss adapter (7 persisted)` · `recorded transit: 2 paths` · 41 fixtures, then
  `tools/demo-site` wrote 52 reference pages and the hygiene gate passed over 251 files.
  Spot-checked: comms' titles are its extractor's output from the served HTML, scouting's
  rows carry `source: rss:demo-origin`, transit's journey is the origin's ICE 331 with
  `reliability: null` because punctuality is absent — the declared degradation.
- [x] ISC-7 — transit's endpoints are env-overridable, so the demo can point the real parser
  at a stub. Three, not the two this claim named: `SJEL_TRANSIT_DBNAV_FAHRPLAN_URL` had to
  join them once dbnav became the default, or the default backend would have been the one
  path a stub cannot reach. Evidence: `every_endpoint_can_be_pointed_at_a_stub_and_otherwise_is_bahn_de`,
  plus a live CLI search against `tools/demo-origin` returning three parsed journeys on both
  backends.

Correction carried over: `demo.toml`'s `[absent.comms]` blames `sources/rss.rs`, which
is *scouting's* file, not Comms'. That reason was written from a bad reading of an `ls`
whose error was silenced.

### F2 · Upstream drift

Why: an upstream that moves and a deployment that does not is the drift worth watching. Q77
(2026-09-02) reversed the answer rather than the question: nothing is held at a version any more,
so the risk is no longer "the bump is late" but "the bump landed and broke something", and what
has to be visible is the pull request, the alert and the receipt. Q109 (2026-09-22) adds one
narrow age window rather than a version pin: Bun/npm resolutions wait 24 hours, the two UI trees
run Socket's scanner, and Cargo/actions keep the zero-day path.

- [x] ISC-8 — every entry a Dependabot pull request or alert names as behind or vulnerable
  is either merged or has a written reason it is held. Falsifier: an open Dependabot pull
  request older than a week with neither. (2026-08-19, measured by the since-retired
  `tools/upstream-checker`: 76 entries · 50 ok · 17 n/a · 9 warn · 0 fail, every warn inside
  its cooldown hold, where waiting was the action. 2026-08-28, PRD Q41 deleted that checker
  and named `renovate.json5` its replacement; the Renovate GitHub App was never installed, so
  the claim then ran for five days with no instrument at all — measured 2026-09-02, the
  repository has zero Renovate pull requests and zero Renovate issues over its whole life.
  2026-09-02, Q74: `renovate.json5` is deleted and `.github/dependabot.yml` replaces it.
  Dependabot needs no App, so the claim has an instrument for the first time — and the hold
  half is gone with the cooldown, so "held with a reason" now means a deliberate refusal,
  never a timer. 2026-09-02, Q77: Dependabot pull requests carry `--auto --squash`
  (`.github/workflows/dependabot-automerge.yml`), so the ordinary bump merges itself once the
  required checks pass and this claim is about the exceptions only. 2026-09-22, Q109: the
  two Bun blocks carry `default-days: 1`, while Cargo and github-actions remain at 0; each
  resolving UI tree carries `minimumReleaseAge = 86400` and
  `@socketsecurity/bun-security-scanner`, and CI runs `bun pm scan`. The scanner is free-mode
  network-backed and therefore an operational dependency, not a replacement for the hold.
  Done 2026-09-30: all 11 queued pull requests merged after `self.json` schema 2 and undici 8.10.2
  greened `main` (d49fd70e, 016d9251, eac7c93b, d25a7353, 8d43b981, 0451bc7a; 0 open PRs). The
  single open Dependabot security alert (#1, glib 0.18.5) was dismissed on 2026-09-29 with the
  documented `osv-scanner.toml` rationale: Linux-only via `gtk 0.18 <- muda/tao <- tauri`, not compiled
  on macOS/iOS and unbuilt in CI. Evidence: `gh pr list` returns 0; `gh api repos/larsboes/Sjel/dependabot/alerts`
  reports 0 open alerts.)
- [x] ISC-9 — the postgres 17.9 → 17.10 image decision is made on its own, not ridden
  along with another change. Falsifier: the bump appears in a commit about something else.
  Closed 2026-08-27 by PRD Q45, which retired the image rather than bumping it: the running
  17.9 instance is read once by `tools/migrate-pg-to-sqlite` and then stopped, so a 17.10
  decision has no subject. The constraint held either way — the retirement is its own
  commit, and the version that ran is in `upstreams.toml`'s git history at that date (Q77
  deleted the field on 2026-09-02).
- [x] ISC-26 — `tools/self check` passes in a fresh clone and `tools/self generate` succeeds on a
  machine with no code graph, so the gate CI runs is a gate CI can repair. Falsifier: a
  `git worktree` of origin/main exits 1 from `tools/self check`, or
  `SJEL_SELF_GRAPH=<a path that does not exist> tools/self generate` exits 1. Done 2026-09-29:
  the per-unit `code` counts and the `graph` block were committed but rolled up from git-ignored
  `graphify-out/`, so on a graphless machine `check` narrowed its comparison to the tracked-file
  layers while `generate` refused to write — the drift it reported was the one drift nobody there
  could fix, and `main` was red on CI's `bun test`, `repo gates` and Pages' `build the page`
  behind it while ten armed Dependabot pull requests older than a week queued (ISC-8). Both
  layers are fused on read now: `self.json` schema 2, `status` and `explain` still show the counts
  where a graph exists, and `tools/generate-site.ts` already refused to publish them. Evidence:
  in a worktree of origin/main, `tools/self check` exits 0 and `tools/self.test.sh` passes every
  case; with `SJEL_SELF_GRAPH` pointing at nothing, `generate` writes schema 2 carrying no `code`
  and no `graph` key.

### F4 · The documents a stranger reads

Why: the top of the README is the product definition, and the sources behind the design go into one
curated place. The README is still 870 lines of doctrine below 230 of product.

- [x] ISC-14 — the README opens like a large open-source project (Graphify, Ollama): logo,
  badges, a screenshot of phone and dashboard showing travel and people first (the areas a
  stranger meets first), what it does, a three-command start, and a
  table of measured results (pseudonymizer 48/48, redaction recall 100% on the frozen corpus,
  Feed ranking 0.941 pairwise, the 1.5 s Same Wi-Fi fallback). Falsifier: a number in that
  table without a command or file that reproduces it. Done 2026-09-27: text wordmark (no logo yet), badges, the
  travel hub on desktop and phone from the live demo, What it does, Quick start, and a Measured
  table whose redaction rows were re-run that day. The 1.5 s fallback left the table: it is a
  setting, not a measurement.
- [x] ISC-15 — `research/` states why the project exists: the problem it answers and the
  sources that show the problem is real. It grows over time; the first version holds at least
  one sourced entry, and the demo site shows it. Falsifier: an entry without a source, or a
  claim its source does not support. Done 2026-09-27: `research/why-sjel.md` with four sourced
  claims and a list of what the sources do not show; `tools/generate-research.ts` renders it at
  `/research` on the demo site. Moved 2026-09-30: the dashboard renders `research/` as its own
  `/research` routes (`dashboard/src/routes/research/`), with a Projects view built from
  `systems.toml` and `upstreams.toml`, and `tools/generate-research.ts` is deleted.
- [x] ISC-16 — the engineering doctrine lives in `CONTRIBUTING.md`, and no link points at a
  README anchor that no longer exists. About 208 links point into it today. Falsifier:
  `git grep "README.md#"` finds an anchor missing from `README.md`. Done 2026-09-27
  (b88ad6d6): 124 files now cite `CONTRIBUTING.md#<section>`. Five older citations of deleted headings now
  point at the sections that replaced them. `upstreams.toml` keeps `pins-and-cooldown` on purpose:
  those strings record adoptions decided under that rule.

### F7 · What fills the disk is visible where it is managed

Why: `tools/storage` already measures this. Its four verbs are `report`, `apply`, `target`
and `prune`; the first three emit `--json`, and it enforces PRD §9's R6, ratified as Q53 on
2026-08-28: build artifacts are not state, and `target/debug` may not exceed `target/release`
by more than 3×. What is missing is any surface a person looks at.

Measured 2026-09-30: this machine held 400 GB of 460 GB with 34 GB free, and nothing in the
dashboard said so. The largest item was invisible to the tool that exists to find it:
`dashboard/src-tauri/target` had reached 15 GB because Xcode's build phase launches `cargo`
without the shell environment, so `CARGO_TARGET_DIR` is unset and cargo falls back to the
local target dir. `prune --target` runs `cargo clean`, which honours that same variable, so it
could only ever clean the one directory. The user-facing gap and the tool's blind spot are the
same gap, which is why they are one feature.

Placement, per `Packs/harness/skills/sjel/references/on-placement.md`: the measurement stays
in `tools/` ("Repository identity, install, wiring, or operator machinery"), because storage
is operator machinery rather than a bounded domain. No new capability and no new route —
`capabilities/sjel-status` already serves a host-watch finding for "a filling disk", and
`dashboard/src/routes/systems/+page.svelte` is already the machine-state page. The
change is two routes in its `service.toml`, one handler, and one panel.

A machine-wide `[build] target-dir` in `~/.cargo/config.toml` was tried on 2026-09-30 as the
quick fix for the recurrence and reverted the same day. It does stop Xcode recreating the
nested tree, but it also redirects *every* cargo build on the machine, including the
checkout's own — so a workspace build would land in `~/.cargo-target` while the supervisor kept
running, and restarting, the stale binaries in `<repo>/target`. The deployment directory and
the build output would have quietly diverged. Visibility in the tool is the fix instead,
because it names the directory rather than moving where things are written.

- [x] ISC-31 — the Systems page shows what fills the disk, from the tool's own `--json`.
  Falsifier: the page renders no storage panel, or `sjel storage report --json` exits non-zero.
  Probe: `sjel storage report --json` and the page. Evidence, 2026-09-30: the report exits 0 and
  `GET /api/sjel-status/storage` returns it unchanged, a non-zero exit carried as data because
  that is how the critical state arrives. The panel rendered in Safari at 1512×868 through the
  dev server: state OK, `326.0 GB used of 460.4 GB`, `107.8 GB free`, the measured classes
  largest-first (`rust-workspace-target` 1.9 GB, report-only; `package-manager-caches` 430 MB),
  `430 MB reclaimable — sjel storage apply`, and six protected paths each carrying the tool's own
  reason plus `4 of these could not be read by the measuring user`. Every figure equals the
  tool's own text and JSON, and `used` moved between two loads (322.8 GB, then 326.0 GB), so the
  panel reads the disk rather than a fixture. It is read-only by decision.
- [x] ISC-32 — `tools/storage` sees a Cargo target dir that is not `CARGO_TARGET_DIR`.
  Falsifier: after an Xcode-driven build, `sjel storage target` reports one path while a second
  target dir exists on disk. Probe: build via Xcode, then `sjel storage target`. The checkout's
  own `target/` is excluded from the answer on purpose: it holds the binaries the supervisor
  runs, and `tools/cargo-hermetic` refuses to point `CARGO_TARGET_DIR` inside the checkout for
  that reason. Only a *nested* workspace — `dashboard/src-tauri/target` — is an escapee.
  Evidence, 2026-10-01: built in b8966e17 with three unit tests. Live probe against this
  checkout: a 5 MB `dashboard/src-tauri/target` stamped with cargo's own `CACHEDIR.TAG`, then
  `sjel storage target` printed `5 MB in 1 target dir cargo does not resolve to here` with the
  `cargo clean --manifest-path` that removes it, and `--json` carried it under `secondary`. The
  directory was planted, not produced by an Xcode build, so the falsifier's own setup is still
  unexercised.

### F10 · An agent writes, under a mode the owner sets

Why: F8 and F9 made agent access read-only. The principal ruled on 2026-10-01 that an agent
writes too, "native" through MCP tools, under a mode chosen per capability: read-only, ask or
auto. The default is auto. A small on-device model may later sort writes into safe and risky,
and is not part of this feature.

Rulings, principal, 2026-10-01:

- One mode per capability, set on the Systems page. No file is edited by hand.
- In ask mode the write waits for Allow or Deny in the menu-bar app (`apps/mac`), and the
  dashboard lists it too.
- The default is auto.
- Pseudonym tokens in an agent's write are turned back into the real values before the write
  reaches the capability, from the same session that issued them.
- Every agent call is logged: time, capability, method, path, status and the gate's decision.
  Never a body.

Product Rule 5 still applies inside auto ([product rules](CONTRIBUTING.md#product-rules)): "A
change that leaves Sjel, or cannot be undone, asks." A route that does either declares it in its
manifest, and the gate asks for it whatever the mode.

Where the files live: the policy, the approvals and the call log are under the overlay's
`secrets/agent/`, which the managed agent policy denies, because an agent that could edit an
approval would allow its own write. A readable copy of the modes and the gate registry are under
`data/`; they decide which tools are offered, never what the gate admits.

Placement: the gate stays in `libs/sjel-server`. sjel-status owns the policy, the approvals and
the call log, because it already owns the Systems page. The MCP server is operator machinery in
`tools/`. sjel-status itself never admits an agent: it starts and stops the machine's services.

- [x] ISC-46 — the mode is per capability, starts at auto, and is changed on the Systems page.
  Falsifier: changing it needs a file edit, or an agent `POST` to a read-only capability reaches a
  handler. Probe: gate tests, and the page. Done 2026-10-01: the page lists calendar, comms and
  devices at `auto`. Setting devices to Read only there survived a reload, wrote
  `data/agent-modes.json`, and cut `sjel mcp`'s devices tools from ten to the four GETs; it was
  then set back to Auto. `the_agent_token_cannot_write_or_read_what_it_cannot_rewrite` refuses a
  read-only `POST` with 403 (65 of 65 `sjel-server` tests pass). Two faults found on the way:
  the served `dashboard/dist` predated the panel until rebuilt, and the running sjel-status was
  an image older than `target/release/sjel-status` (inode 614898429 against 615154997), so it
  wrote the policy but not the copy until restarted.
- [x] ISC-47 — in auto mode an agent write reaches the handler, and a route declared `confirm`
  asks anyway. Falsifier: an agent moves mail to Trash with no approval. Probe: gate tests.
  Done 2026-10-01: `a_confirm_route_asks_even_in_auto_and_one_approval_admits_one_write` and
  `in_auto_a_write_reaches_the_handler_with_its_tokens_restored_and_is_logged` in
  `libs/sjel-server/src/lib.rs`. comms declares ten confirm routes (`AGENT_ROUTES` in
  `capabilities/comms/src/server/main.rs`): the Gmail actions, the cloud approval and run,
  `/ingest`, redaction, and setting a data class by hand, because an agent that lowered a class
  could then read what it withheld. Every `serve_local` capability now admits the agent under
  its mode, keyed by its registry name; sjel-status does not.
- [x] ISC-48 — an approval is single-use and bound to the method, the path and a digest of the
  body. Falsifier: one approval admits a second write, or a different body. Probe: unit tests.
  Done 2026-10-01: `an_approval_admits_its_own_write_once` in `agent_policy.rs` and the gate
  test above. A claim is a file rename, so two requests racing for one approval admit one.
- [x] ISC-49 — tokens in an agent's write body become the real values before the handler, and a
  token the session never issued is refused. Falsifier: a handler receives `<PERSON_…>`. Probe:
  gate tests. Done 2026-10-01: `a_token_the_session_never_issued_is_refused` and the auto test
  above. A limit, measured in the design and not yet answered: keyed tokens are one-way, so only
  the capability process that issued a token can restore it. A token read from comms and written
  to trips is refused with a message that says so.
- [x] ISC-50 — every agent call leaves one log row with no body, and the Systems page shows the
  latest. Falsifier: an agent call with no row. Probe: gate tests and the page. Done 2026-10-01:
  one `devices__get_api_devices` call through `sjel mcp` appeared under Latest calls as time,
  `devices`, `GET /api/devices`, `200`, `read`, with no body, beside the `GET /routes` rows from
  tool discovery. The gate test is
  `in_auto_a_write_reaches_the_handler_with_its_tokens_restored_and_is_logged`.

### F12 · A shared link becomes a row, and a garment is a row like any other

Why: the Home domain was written as **one table for everything the household owns** — *"a tent
and a wardrobe are the same row shape"*, Q58, 2026-08-30 — and `interior_item` has carried the
equipment columns since B51. What it never had was an **intake**. A row entered by one of two
doors: `interior import` from `inventory/*.toml`, which describes furniture and fills none of
the seven gear fields, or the dashboard's create form, which offers `piece`/`slot`, five numbers
and a price. So "put this in my wardrobe" meant retyping a shop page by hand, and the old PRD's
own backlog said so: *"Item intake: scan an object, share a link, one wishlist."* This builds
the link half, and the three fields a garment needs beyond furniture.

Placement: no new table, no new capability and no new route. Three columns on `interior_item`
and one CLI verb, because Q58 already answered where the row goes and the answer was *there*.

- [x] ISC-61 — `interior wunsch <url>` creates a `wanted` row in the same table with the link on
  it, the title from the page, and a price **only where the page declares one**. Falsifier: a row
  carrying a price the page never declared, a non-EUR amount carried as though it were EUR, or a
  fetch that reaches `file://` or a loopback address. Probe: `cargo test -p sjel-interior wunsch`, then
  `interior wunsch <url>` against a live page and `interior inventory`. Evidence, 2026-10-05:
  12 unit tests in `capabilities/interior/src/wunsch.rs` — a number in prose is not a price, a
  declared price is read *and named*, `itemprop` is the third door, both attribute orders parse,
  a non-EUR currency is refused, and `ab 79` is not a price. Live: `interior wunsch
  https://example.com --category kleidung --groesse M` created `example-domain` with the title
  from `<title>`, reported the missing price, and a second run made `example-domain-2` rather
  than overwriting. `file:///etc/passwd` and `http://127.0.0.1:8092/api/inventory` exit 2 with
  the guard's own words and write nothing. The first version of `betrag_cent` failed this
  claim's own falsifier — `trim_start_matches(is_alphabetic)` turned `ab 79` into 79,00 € in a
  column that sums into the wishlist total — and the module header asserted the opposite while
  every test passed, because none named `ab 79`. **Live, against a real shop** (2026-10-05, a
  Uniqlō product page): the first run fetched nothing and still wrote a row labelled `00`,
  because the fetch failure was a warning and the URL's last path segment became the name — the
  fallback is gone and a row now needs a title or `--label`. The same run showed *why* it fetched
  nothing: that shop answers the Sjel user-agent with `HTTP/2 stream 1 was not closed cleanly:
  INTERNAL_ERROR` and a timeout over HTTP/1.1, and 200 with 1,1 MB to a browser agent. `wunsch`
  now sends one, named and reasoned at the constant, as `scouting/adapters/meetup.rs` and
  `transit/hafas.rs` already do. After the fix the same URL produced
  `ultra-stretch-hose-fur-herren-uniqlo-de` with the title from `og:title`. NOT verified: a
  shop page that declares a price — that page declares none (its price sits in embedded app
  state beside four other amounts, which is a guess in the column that sums into the wishlist
  total), so the declared-price path is still covered by unit tests alone.
- [x] ISC-62 — a garment is a row in `interior_item` with `category`, `groesse`, `farbe` and
  `saison`, and a file that predates the columns gains them without losing a row, a state or a
  placement. Falsifier: the columns cannot be set or corrected from `/interior`, or an existing
  `interior_item` loses a row, a state change or a placement when the migration runs. Probe:
  `cargo test -p sjel-interior --test kleidung`, then the create and edit forms. Evidence, 2026-10-05:
  three tests in `capabilities/interior/tests/kleidung.rs` build a file in the shape that was on
  disk before the change (wide `kind` CHECK, no clothing columns), assert the row, its state and
  its placement survive, that a garment round-trips, and that a second start does not rebuild the
  table and blank the columns. `dashboard/src/lib/api.ts` and `routes/interior/+page.svelte`
  carry the five fields in **both** forms, because the edit form is where a wrongly read title or
  price gets corrected; `svelte-check` reports 0 errors. `PATCH /api/items/:id` needed no change:
  `merge_patch` validates against `Item`, not a second list of names. The live overlay's own file
  opened with the columns and kept its 47 rows (29 `piece`, 18 `slot`). NOT verified: the forms
  clicked in a browser, because every route here needs the operator's credential and an agent
  session is refused one by design (the same gap ISC-55 records).
- [x] ISC-63 — the branch a row belongs to is a word the data uses, not a list declared in code,
  and two spellings of one branch are named instead of merged. Falsifier: a hardcoded branch list
  in the dashboard, a category silently lowercased or rewritten on write, or two spellings of one
  branch that nothing reports. Probe: `cargo test -p sjel-interior --lib`, then `interior inventory`
  against a database holding `kleidung` and `Kleidung`. Evidence, 2026-10-05: `store::kategorien`
  reads the distinct branches with their counts and `store::kollisionen` groups the ones that
  differ only in case or whitespace, with three unit tests in `store.rs` (two spellings reported,
  a unique branch not, `Kochen`/`kochen` a collision while `kochen`/`kochen-und-backen` is not).
  Measured: a scratch database with `Kleidung`, `kleidung` and `wohnzimmer` printed both branches
  and the group `Kleidung | kleidung`, while a ` kleidung ` written beside a `kleidung` collapsed
  into one row — the query trims, and the function reports only what trimming does not already
  merge. The dashboard's suggestion list is `$derived` from the loaded rows; the five-word list
  written into Svelte in the first version is gone, because it was the second truth this claim is
  about. NOT verified: a branch collision in the live overlay. It has 47 rows and **no**
  `category` on any of them, so the branch list is empty today and the first word is the
  principal's to type.

### F13 · The inventory stops living behind the floor plan

Why: measured 2026-10-05. `interior` holds two domains in one process — the geometry of a flat
and the things the household owns — and only one of them may run at boot, because the other
serves a floor plan and photographs of a home. So `interior` is the only capability in this
repository that is `autostart = false` **and** consumed by another: `capabilities/trips` reads
`GET /api/inventory` over HTTP on a 3 s timeout (`trips/src/server.rs:1528`) and answers
`interior_reachable: false` when nothing is listening, while `trips/service.toml` declares no
`requires` for it. The consequence is not theoretical: after a restart, until somebody opens
`/interior`, every pack list loses the weight and every equipment attribute of every item on it.
Two tests in `trips/src/pack.rs` cover that degraded path, which is how it stayed invisible —
the behaviour was tested, the *coupling* was not declared.

The doctrine already names the fix. *"Capability owns a bounded domain, external system or data
store"* — and a data store is not a `libs/` crate, because libs own no domain. Q58 named it
earlier still: *"when gear lands, rename it then, do not fork it"*, and gear landed at B51.
`requires` is not a substitute: `up` filters on `autostart == "true"`
(`tools/sjel-cli/src/runner.rs:198`), so declaring it orders the enabled set and pulls a
dependency in on `enable` — it starts nothing.

**The split, and the line is "does it need a room".**

| stays in `interior` | moves to `inventory` |
|---|---|
| `room.toml`, `rules.toml`, `layouts/*` | `inventory_item`, `inventory_item_state` |
| `interior_placement` — where a thing stands | `import` (`inventory/*.toml` is the migration source) |
| clearance, search, compose, plan, einbringung, sonne, toleranz | `wunsch` — the link intake |
| `deklaration`, `kaufen` — both need the layouts | the item HTTP surface, the wishlist, `kategorien` |
| the floor plan, `roomplan`, `media` | `obsidian` vault writeback |

`budget.rs` splits with it: `monatssaldo` (reads finance) goes, `kaufreihenfolge` stays because
it asks which layouts already build on a need. `interior` keeps reading the items from the
**shared file** rather than over HTTP, the named exception `budget::monatssaldo` already
establishes for `finance_transaction_projection` — because `interior check` is used as a gate
and must keep working with no service running at all.

- [x] ISC-64 — `inventory` is a capability of its own, `autostart = true`, owning the item tables
  under its own prefix; `interior` reads them from the shared file and keeps serving the floor
  plan on demand. Falsifier: after a restart with nothing opened, a pack list still reports
  `interior_reachable: false`; or `interior check` needs a running service. Probe: restart the
  host, then `trips`' pack endpoint and `interior check <layout>` with nothing else started.
  Evidence, 2026-10-05: `capabilities/inventory` on port 8101, `autostart = true`, its own
  persistence unit installed (`tools/service-runner.sh persistence` → `installed`), and the only
  writer of `inventory_item`. Measured: with `interior` **stopped**, `GET /api/inventory` on
  8101 still answers with all 47 rows, while 8092 answers nothing — which is the defect this
  feature exists for, inverted. `trips` points at 8101 in both call sites
  (`src/server.rs`, `src/interior_client.rs`) and declares `requires = ["inventory"]`, so `up`
  starts the dependency first. `interior` reads the rows through the shared file under a second,
  named prefix (`ITEM_PREFIX`), so `interior check` still runs with no service at all. NOT
  verified: the host actually rebooted. The claim rests on the launch unit being installed and on
  `up` ordering, not on an observed restart.
- [x] ISC-65 — the item surface answers on inventory's port, `trips` names it and declares
  `requires`, and no second capability writes `inventory_item`. Falsifier: two writers of one
  table, or an item route still mounted by `interior`. Probe: `rg 'api/items' capabilities/`, and
  `interior`'s own `ROUTES` manifest (the coverage test refuses an undeclared route both ways).
  **Half done, and this is the honest state.** The surface answers on 8101 and `trips` names it.
  `interior` still mounts `POST /api/items`, `PUT`/`PATCH /api/items/:id` and
  `POST /api/items/:id/state` — they write the **same** table, so nothing diverges, but there are
  two writers of one store and the falsifier still fires. What is left is a deletion: those three
  routes, `api_inventory`, `api_wishlist`, `api_vault_writeback`, the `import`/`inventory`/
  `wunsch`/`vault-writeback` verbs, and `interior/src/{import,wunsch,obsidian}.rs` — all of which
  now exist, working and tested, in `capabilities/inventory`. `interior/src/store.rs` also keeps
  its write half (`upsert_item`, `record_state`, `update_item_if_revision`, `sync_operation_*`)
  only for those routes. Removing them is the next landing and touches no data.
  Done 2026-10-08, read from the tree: `interior` mounts no item write route. The one item route
  left is `POST /api/items/{id}/impact`, which computes and writes nothing
  (`capabilities/interior/src/api.rs:1051`). `import.rs`, `wunsch.rs` and `obsidian.rs` are gone
  from `interior/src`, and `rg 'fn (upsert_item|record_state|update_item_if_revision)'
  capabilities/interior/src` finds nothing. The deletion landed in b10e5260 itself. The paragraph
  above was written before that commit, and nobody ticked the box afterwards.
- [x] ISC-66 — the live rows move without loss, and the move is a migration rather than a
  re-import. Falsifier: a row, a state change or a placement missing after the move, or a state
  history invented by it. Probe: `cargo test -p inventory --test umzug`, which builds a file in
  the pre-move shape and asserts every row, every `since` and every placement survives; then
  `interior inventory` against the live overlay, which held 47 rows (29 `piece`, 18 `slot`) on
  2026-10-05. Evidence, 2026-10-05: five tests in `capabilities/inventory/tests/umzug.rs` — rows,
  two state changes in order, the placement, the revision, and that the old table is **gone**
  (`interior_item` and `interior_item_state` no longer exist) while `interior_placement` keeps its
  rows with **no** foreign key left. Run against a copy of the live file: 47 items → 47, 48 states
  → 48, 0 placements → 0, revisions 1..2 preserved, and the open need read **582.96 €** on both
  sides of the move — the same number `interior inventory` prints from the unmigrated file.
  The move also runs on a real start: the live database was migrated by the capability's first
  boot, and `interior` reads it from then on. `gear_migration.rs` and `kleidung.rs` were interior's
  tests of that migration and are succeeded by `umzug.rs`; the case they covered that is *not*
  obvious — a file older than B51, with a narrow `kind` CHECK — is `eine_aeltere_datei_verliert_
  durch_den_umzug_nichts`, which copies only the columns both tables have.

**Still open after this landing, and deliberately not hidden:** `interior` mounts the item routes
it should have given up (ISC-65), the dashboard's item surface still lives on `/interior` with
`/inventory` as a redirect stub, and `/api/sync` and `/api/media` — the phone's mutation path and
the item images — were left in `interior` because moving a signed, device-authenticated ingress
without verifying it is how a silent loss happens.

Sequencing, because a half-moved store is the failure this repository has already paid for once
(B51 was reverted in `815750c` rather than land half a form in a live file): the migration and
the switch-over are **one** landing, since a copied table and a live table are two truths about
the same thing. Started and landed 2026-10-05 in the order the file records.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| ISC-1 | command | `gh issue list --state open`, here and in the overlay | empty | gh | stated goal |
| ISC-2 | file | read each closing comment and its named ISA | content present | Read | "carry through ISAs" |
| ISC-3 | command | `rg -i "backlog is Issues"` over the four files | zero hits | rg | "no new issues" |
| ISC-4 | command | `rg "gh issue create" .github/workflows` | zero hits | rg | "no new issues" |
| ISC-5 | command | `cargo test --workspace --locked` | all pass | cargo | C1 |
| ISC-6 | command | demo build, inspect recorded responses | three capabilities present | bash | F1 |
| ISC-7 | code inspect | read transit's URL consts | env-overridable | rg | F1 |
| ISC-8 | queue | `gh pr list --author app/dependabot` | every open entry merged or held with a written reason | gh | F2 |
| ISC-9 | command | `git log` for the postgres retirement | its own commit | git | F2 |
| ISC-31 | command | `sjel storage report --json`, then the Systems page | panel present, exit 0 | jq, browser | F7 |
| ISC-32 | command | build via Xcode, then `sjel storage target` | every target dir listed | bash | F7 |
| ISC-61 | command | `cargo test -p sjel-inventory wunsch`; `inventory wunsch <url>` then `inventory inventory` | a `wanted` row, no undeclared price | cargo, bash | F12 |
| ISC-62 | command | `cargo test -p sjel-inventory`; the create and edit forms | rows, state and placement survive | cargo, browser | F12 |
| ISC-63 | command | `cargo test -p sjel-inventory --lib`; `inventory inventory` on a database holding `kleidung` and `Kleidung` | both named, none merged | cargo, bash | F12 |
| ISC-64 | command | restart the host; then trips' pack endpoint and `interior check` | pack list resolves items, check runs with nothing started | bash | F13 |
| ISC-65 | code inspect | `rg 'api/items' capabilities/` and `interior`'s `ROUTES` | one writer, no item route in interior | rg | F13 |
| ISC-66 | command | `cargo test -p sjel-inventory --test umzug`, then `interior inventory` on the live overlay | every row, state and placement survives | cargo | F13 |

## Log

- 2026-10-05 · **`inventory` is its own capability, and the item rows left the floor plan's
  process** (F13). Landed in one pass because a copied table beside a live one is two truths
  about the same thing: `capabilities/inventory` (port 8101, `autostart = true`, persistence unit
  installed) now owns `inventory_item`, `inventory_item_state`, the item surface, `import`,
  `wunsch` and the vault writeback; `interior` reads the rows through the shared file under a
  second named prefix and keeps the geometry, the rules, the layouts and `interior_placement`.
  Four things were found by running it rather than by reading it. **(1)** The migration had to
  rebuild `interior_placement` before dropping the item table: it carries `ON DELETE CASCADE`,
  and `DROP TABLE interior_item` would have taken every placement with it — the same trap
  `widen_kind_check` documents. The new table has **no** foreign key, because the rows now belong
  to another capability and a cross-prefix FK would make one capability's deletes fail on the
  other's rows. **(2)** Placements are not inventory's at all: the copied store created
  `inventory_placement` and read from it, so the migration test found zero placements — they
  belong to `interior`, and inventory touches them exactly once, in the move. **(3)** `interior`
  queries `catalogue()` through a prefix constant, and that one function was the one I forgot:
  every route answered `no such table: interior_item` until it was switched too. **(4)** Deleting
  the item DDL from `interior`'s migration left its CLI and its tests with no table at all, so
  `interior` keeps a `CREATE TABLE IF NOT EXISTS` bootstrap with the current column set and no
  ALTER chain — inventory owns the chain. Measured: against a copy of the live file, 47 items →
  47, 48 states → 48, revisions preserved, old tables gone, and the open need read **582.96 €**
  on both sides of the move; with `interior` stopped, `GET /api/inventory` on 8101 still answers
  and 8092 answers nothing. NOT verified: the host rebooted, and the phone's `/api/sync` and the
  item images, which stayed in `interior` on purpose. NOT finished: `interior` still mounts the
  item routes it should have given up (ISC-65), so there are two writers of one table — the same
  table, so nothing diverges, but it is a deletion left for the next landing.

- 2026-10-05 · `interior wunsch <url>` and three clothing columns, recorded as F12. A shared link
  becomes a `wanted` row in `interior_item` — the same table a sofa lives in, per Q58 — with the
  title from the page and a price only where the page declares one (`og:price:amount`,
  `product:price:amount`, `itemprop="price"`). Three things were found by running it rather than
  by reading it. **(1)** The first run panicked: `sjel_http::client` is blocking, `main` is
  `#[tokio::main]`, and a blocking reqwest client dropped on a runtime worker panics at
  `tokio/src/runtime/blocking/shutdown.rs:51` — `libs/sjel-http/README.md` says so in its own
  heading, and the fetch now goes through `tokio::task::spawn_blocking`. **(2)** A *refused* URL
  still wrote a row: `file:///etc/passwd` produced an entry named `passwd`, because the fetch
  failure was a warning and the URL's last path segment became the label. A guard refusal is a
  decision and a timeout is a circumstance, so `wunsch::Abruf` now separates them and only the
  second is survivable. **(3)** `betrag_cent` turned `ab 79` into 79,00 € —
  `trim_start_matches(is_alphabetic)` cut the `ab` — in the one column that sums into the
  wishlist total that joins to `finance`. The module header asserted the opposite and every test
  passed, because none of them named `ab 79`; the reader is now strict about what may sit beside
  the digits, and four tests name the case. **(4)** Run against a real shop the next day, the
  first attempt wrote a row named `00`: the page could not be fetched, the failure was a
  warning, and the URL's last path segment became the label. Two things came out of that one
  observation — the fallback is gone (a row needs a title or `--label`), and the reason the
  fetch failed is that the shop refuses a self-identifying client at the HTTP/2 layer while
  answering a browser agent with 200 and 1,1 MB. `wunsch` now sends a browser agent, the third
  such case in this repo after `scouting/adapters/meetup.rs` and `transit/hafas.rs`, named and
  reasoned at the constant rather than left as a mystery in a header map. A third round of
  questions settled what the first two had left open, and all three recommendations were
  accepted: the umbrella keeps its (mis)name and the branch is read from the data, a garment is
  `kind = piece` and not `gear`, and the fetch stays in the CLI. The branch half was the only one
  that needed code: `store::kategorien` and `store::kollisionen`, printed by `interior inventory`,
  and the dashboard's suggestion list derived from the loaded rows instead of the five words the
  first version had written into Svelte — the same second-truth mistake this change is about, made
  by me, one file away from the fix. Verified: 18 test binaries green, 78 lib tests
  (12 new in `wunsch.rs`, 3 in `store.rs`), 3 new in `tests/kleidung.rs`, `svelte-check` 0 errors,
  `cargo clippy -D warnings` clean, `sjel gates` 17/17, `sjel test` 5/5, and the live overlay's
  file migrated to the columns with its 47 rows intact. NOT verified: a shop page that declares a
  price, the forms clicked in a browser (the credential boundary ISC-55 already records), and a
  branch collision in the live data — it has 47 rows and no `category` on any of them, so the
  branch list is empty and the first word is the principal's to type.

- 2026-10-04 · `tools/model-check` is Rust, and `tools/doctor` no longer starts an interpreter to
  check this machine's inference roles. That leg ran `bun tools/model-check.ts --local --json`
  unconditionally and measured 6.9 s of the doctor's 20.6 s; the doctor now calls
  `src/model_check/` in-process and reads the payload it read before, field for field, so the
  section is unchanged and the whole run fell to 11.8 s. It was the last place a Rust tool started
  an interpreter to do work it could do itself, and the readers were already Rust: `libs/inference`
  owns the registry, and `is_loopback_url`, `resolve_key_file` and `api_key_from_file` now come from
  there rather than from copies — the first two made public for this caller, `resolve_key_file`
  extracted from the resolver that already lived there. Verified against the TypeScript before
  deletion, both implementations pointed at one stub provider through a scratch overlay written for
  the comparison, so nothing real was touched: five runs — `--local`, `--local --json`, `--json`,
  `--probe --json` and `--probe` — matched byte for byte in stdout, stderr and exit code for every
  text output, and the JSON payloads are equal once parsed, entry for entry and in the same order.
  Two differences are deliberate: JSON object keys are sorted, as every port since `harnesses`
  records, while the `entries` ARRAY keeps the file's order through a local `OrderedMap`, because
  `serde_json`'s map is a `BTreeMap` here and `preserve_order` would change it for every consumer of
  this binary (the same reason `updates/parse.rs` carries its own); and the credential is read the
  way `libs/inference` reads it, which changes behaviour for a backend whose key file names a `~/`
  path or a JSON settings file — the TypeScript joined `~/.omlx/settings.json` onto the config
  directory, found nothing, and reported the role unprovisioned without ever dialling it. No
  loopback role on this machine declares one, so nothing it reported changed. A role's declaration
  is still read leniently rather than through `InferenceConfig`, which requires `backend` and
  `model` on every role and degrades the whole config to empty when one is missing — that would be a
  broken declaration reading as doctor's `ok`, which is the one outcome this tool must not produce.
- 2026-10-04 · The MCP server's query encoding no longer depends on the order its arguments were
  built in, which was the one red gate in `cargo test --workspace --locked`. `tools/sjel-mcp`'s
  `encode_query` iterated `serde_json`'s map, which is an insertion-ordered `IndexMap` whenever any
  crate in the build enables `preserve_order` — `libs/extraction` does, through `xberg` — so
  `a_query_space_is_a_plus_and_a_path_space_is_percent_20` passed under `-p sjel-mcp` and failed
  under the workspace. Measured before the fix: 2717 passed, 1 failed, and `cargo test -p sjel-mcp
  -p sjel-extraction` reproduced it with `sjel-cli` absent from the set, so it predated the two
  scheduled-job ports it was found beside rather than being caused by them. Pairs are sorted by key
  before encoding, the answer `tools/claude-code-config/src/check.rs` already gives for its digest
  and states the same trap at. Held by a second case asserting that the same arguments in either
  insertion order produce one URL, which is the part a fix that only sorts at one depth would miss.
  After: 2719 passed, 0 failed.
- 2026-10-04 · The two remaining scheduled bun jobs are Rust, and no timer in the repository starts
  an interpreter any more. `tools/feed-sweep.ts` (comms, every 6 h) and `tools/sparpreis-watch.ts`
  (trips and transit, every 12 h) move together, because they are one class of work — a timer that
  starts an interpreter to make a handful of HTTP calls and exit — and because each needed the same
  manifest change. `tools/sjel-cli/src/feed_sweep.rs` is the first; `sparpreis_watch/mod.rs` and
  `pure.rs` are the second, the pure half carrying `tools/sparpreis-watch.test.ts`'s cases as Rust
  unit tests. The manifest-port reader both tools carried a copy of becomes `paths::port_in_manifest`,
  with that file's four port cases. `capabilities/feed-sweep/service.toml` and
  `capabilities/sparpreis-watch/service.toml` now name `target/release/sjel-cli <verb>` with the
  `build` line `host-watch` carries, so service-runner puts `cargo` on the job's PATH; launchd runs
  service-runner and the runner reads the manifest, so no installed unit needs reinstalling.
  Verified against the TypeScript before deletion with one stub server and both implementations
  pointed at it through a scratch root, so nothing tracked was touched. `feed-sweep` ran in four
  scenarios (a rich payload with an unreachable source; an empty body with no `sources` key; a 500;
  a 200 whose body is not JSON) and matched byte for byte in stdout, stderr, exit code and request
  sequence. `sparpreis-watch` ran over one plan carrying a stage, a legacy per-day item to fold and
  a rail `option_set` to match, and matched in all four, its request sequence — both item writes and
  the one delete included — equal once the two runs' wall-clock `observed_at` is normalized. Four
  differences are deliberate: every request carries a timeout (300 s for the scan and for
  sparpreis' calls, 600 s for the relevance page, matching the two `AbortSignal` budgets the
  TypeScript already set and giving sparpreis the one it never had); the deployment credential goes
  on with `sjel_server::InboundAuth::with_loopback_auth` rather than a second implementation of the
  loopback rule; JSON object keys are sorted, as every port since `harnesses` has recorded; and
  `-h` prints the usage, which neither TypeScript read. `self.json` was regenerated in the same
  change — two `cargo-dep` evidence rows for the crate's two new dependencies. The cost is named in
  the crate's `Cargo.toml`: `sjel-http` and `sjel-server` take the binary from 7.3 MB to 14.1 MB,
  every crate of it already in the workspace lock and already built for the capabilities.
- 2026-10-04 · `sjel update` can move an npm finding whose owner is current, which is the class the
  audit's installed pass had no verb for. Measured before the change: all five global npm owners
  reported `✓ current` while 29 findings sat in their subtrees, so the report was right about every
  row and wrong about the machine, and `tools/audit`'s advice (`run tools/audit`, `sjel update`)
  named nothing that reached them. `npm outdated -g` asks the top level; `--all` asks every nested
  node and each entry carries the `location` that says whose tree it is in, so a row per owner now
  names what is behind it — as CURRENT and never as stale, which is the judgement here: every
  nested node of every global tree is behind someone's latest, because a parent pins what it was
  published against, and marking owners stale would make the report red forever on every machine
  with a global install. What was missing was the command, not a flag:
  `sjel update apply --only npm --re-resolve <package>` reinstalls the owner, which is what makes
  npm resolve its ranges again (cargo's half drops `--locked`; npm has no lock to drop). The row's
  note carries it, and `tools/audit`'s installed pass now prints it whenever it finds something.
  Two defects were found and fixed while writing it rather than recorded: the first version called
  a package behind when it was AHEAD of the registry's `latest` tag, naming `accepts 2.0.0 → 1.3.8`
  as its example — a downgrade nobody should make — so the entries are filtered by the crate's own
  `version_newer`, which is the rule the cargo surface already uses; and the existing "named on the
  command line but nothing plans to move it" warning was cargo-only, so it would have called a
  perfectly planned npm name a typo. Held by four `cargo test -p sjel-cli` cases: the nested shape
  in both forms npm emits (an object at one location, an ARRAY of them at several, which the
  top-level parser silently drops), the owner read from the outermost `node_modules` pair — scoped
  owners are two path segments — and the plan, where a stale owner is installed once and not twice.
- 2026-10-04 · The two global MCP servers were removed, which closed the one finding no version
  could move. `@modelcontextprotocol/server-github`'s entry was an EXACT pin: it declares
  `@modelcontextprotocol/sdk: 1.0.1` with no range, so re-resolving its tree cannot move the sdk,
  and npm reports the parent deprecated, so upstream will not republish — its own comment said the
  end was a decision, not a release. The decision was measured, not assumed: nothing on this
  machine registered either server. `~/.pi/agent/mcp.json` holds only `sjel` (Sjel's own
  capabilities, the one MCP surface actually in use), `~/.claude.json` only `graphify` per project,
  `~/.codex/config.toml` only ChatGPT-app binaries, and a grep of `~/.pi`, `~/.agents`,
  `~/.claude`, `~/.config` and `~/Library/LaunchAgents` found both package names only in old
  session transcripts. `server-github` and `server-filesystem` came off with `npm -g rm` (149
  packages) and the exception went with them: 23 entries became 22, the count the file's own
  header had already claimed. The one real use of `@modelcontextprotocol/*` here is the SDK as a
  LIBRARY inside `Packs/harness/pi-packages/pi-web-access` (`baizhi.ts`, `zai.ts`) — a range that
  resolves past the vulnerable version, and not a server, so never the finding.
- 2026-10-04 · The audit's installed-software pass accepts its findings with reasons, and the
  repository gate keeps its own rule. `tools/audit`'s second pass reports software installed
  outside this checkout, where a fix is frequently out of reach: `cargo install --locked`
  resolves the Cargo.lock the crate PUBLISHED — which is also why the scan's model of
  "installed" is accurate — a global npm tree is resolved by whoever published the package, an
  EXACT pin cannot be re-resolved at all, and one package on this machine is on no registry.
  Measured first: 36 findings became 22 when two global npm packages were reinstalled and their
  subtrees re-resolved (`@mariozechner/snap-happy` 12, `@modelcontextprotocol/server-github` 2),
  even though every crate and every top-level npm package involved was already at its latest
  release — those 14 came from dependency ranges re-resolving, not from upgrades. The remaining
  22 are accepted in a NEW file, `osv-scanner-installed.toml`, read by that pass alone. The
  fix is in the split, not in a wider policy: `osv-scanner.toml` keeps "known vulnerabilities
  remain blocking", and CI reads it, so accepting one there would have loosened CI too. Each
  entry names the reach, the fixed version when one exists, and what ends it — four classes:
  inside `npm@12.2.0`'s own dependency tree (11 findings; only a newer npm moves them, and
  `http-cache-semantics` has no fixed version at all), the published locks of crates already at
  their latest (lru via bottom and macmon, rustls via cargo-deny, and tauri-cli's difference,
  rsa, rustls-pemfile and rustybuzz), and three `simple-git` RCEs (9.8, 9.8, 8.1) inside a
  private package whose declared range `^3.28.0` already allows the fix — those carry the
  shortest date in the file, because rebuilding that package is the one class a person can close
  today. The cost is stated in the new file's header rather than hidden: the pass runs with
  `--verbosity error`, so an entry that stops applying is NOT announced, and the header carries
  the by-hand command that lists which still apply. Splitting the config exposed two
  informational advisories the installed pass had been inheriting from the shared file (`paste`,
  `ttf-parser`); they are repeated in the new one so the split did not itself become a finding.
  `tools/audit` exits 0 on this machine now, which is the point: an audit that always fails is
  an audit nobody reads. Two things measured while doing it and left open: `sjel update` cannot
  move this class — every global npm package reports current while 15 findings sat in their
  subtrees, so the audit's own advice named no command that reaches them — and `xberg-cli`
  (1.3.0 -> 1.3.3) and `pnpm` (12.8.1 -> 12.9.1) are stale rows the tool owns and can move.
- 2026-10-04 · The inference-key gateway, vault unlock and Keychain migration move into
  `tools/sjel-cli/src/inference_keys.rs`. The pi-facing catalog stays TypeScript; `bw-key.mjs`,
  the old `tools/bw-unlock` implementation and `keychain-write.exp` are deleted, while their
  launchers keep the command paths. This keeps the secret-handling implementation in one Rust
  process and removes Node, Expect and shell plumbing from those paths. Keychain migration sends
  hex-encoded key material to `security -i` over stdin, never in argv, then verifies the stored
  bytes and removes the plaintext cache. Compared with the old scripts in a scratch environment
  with synthetic keys and stub `bw`/Keychain commands: keychain and vault reads, duplicate-name
  selection, `--check`, `--manifest`, `bwu --status`, and unlock/session caching matched; the new
  migration was exercised separately and verified the write and cache removal. JSON key order
  differs; `status` omits `unlockHelper` and `failure`, which the extension does not read. The key
  timeout is one total lookup budget, not a fresh allowance for each fallback call. The `zeroize`
  crate was already in the workspace lock.
- 2026-10-04 · `tools/discover-ui-packages` is Rust, with the CI and doctor call paths unchanged.
  Its recursive package walk and classifier move to `tools/sjel-cli/src/ui_packages.rs`; the
  launcher still answers both the human report and `--dirs`, and `SJEL_UI_DISCOVERY_ROOT` remains
  the scratch-tree seam. Its 12 planted-tree TypeScript cases are covered by 12 Rust tests,
  including a repository-root package path case. Compared against TypeScript on this checkout and on
  scratch trees: default output, `--dirs`, stderr and exit codes matched, including refused UI
  packages and a tree with no checkable package. Malformed `package.json` still fails closed, but
  the diagnostic text differs between `serde_json` and `JSON.parse`. `serde_json` was already a
  direct dependency, so this added no crate.
- 2026-10-04 · `tools/host-watch` is Rust, and the hourly job stops starting an interpreter. It
  was the last `bun run` job whose readers are already Rust: `sjel-status` serves its rows at
  `/api/sjel-status/host-watch` and the dashboard ranks them at band 900, while `tools/storage
  report --json` and `host-net check --json` stay invoked rather than reimplemented. `src/host_watch/
  mod.rs` holds the verb surface, the policy read, the three probes and the store write; `pure.rs`
  holds the two `ps` parsers, the runaway rule, the storage and net folds and the emission and
  resolution decisions, with the TypeScript test's 34 cases as 31 Rust unit tests. It is also the
  first tool in `tools/sjel-cli` to open the shared store, through `sjel_config::database_path` and
  `sjel_store::{open_pool, write_transaction}` rather than a second declaration of where the file
  is; the crate links bundled SQLite from here on (4.9 MB to 6.9 MB measured, both already in the
  workspace lock). Verified against the TypeScript before deletion with a fake `ps` first on PATH,
  so both saw one frozen process list: `-h`, `--dry-run` and the create/refresh/clear runs were
  byte-identical in stdout, stderr and exit code, the `host_watch_findings` rows left behind were
  identical after all three, a run with no policy exited 2 from both, and `--json` is equal once
  parsed. Then the real paths: `tools/service-runner.sh start host-watch` rebuilt through the new
  `build` line and reported `808 processes, disk ok — nothing to report`, and `sjel-status` served
  `{"findings":[]}` from the table this writer maintains. `capabilities/host-watch/service.toml`
  now names `target/release/sjel-cli host-watch` with a `build` line, the shape `sjel-status` and
  `punctuality` use — that is what puts `cargo` on the job's PATH, which a launcher would have
  hidden — and the installed unit needs no reinstall because launchd runs service-runner and the
  runner reads the manifest. Three differences are deliberate and named in tools/sjel-cli/README.md:
  `--json` keys are sorted (parsed payloads equal, and the camelCase field name is kept), the
  overlay is resolved by `sjel-config` (`SJEL_PERSONAL_ROOT`) rather than by `tools/lib/overlay.ts`
  (which preferred `SJEL_OVERLAY_ROOT`; paths.sh keeps the two equal), and `host-net-cli` is looked
  for under `CARGO_TARGET_DIR` when set, where the TypeScript always looked in `<root>/target`.
- 2026-10-04 · `tools/audit` is Rust, and the audit path no longer needs `bash` or `jq`. It
  closes the loop the `updates` port opened: `sjel update apply` runs it as its final step
  (`updates/report.rs`'s `run_audit`) and `tools/doctor` reads the verdict its exit code put into
  the host-patch receipt, so the 0/1/2 contract already had two Rust readers — which is why it was
  the natural next one. `src/audit.rs` holds the verb surface, the two repository scans and the
  installed-software pass, plus the pure half — purl encoding, which rows reach the SBOM, what
  counts as an inventory, and the exit precedence. `tools/audit.test.sh` keeps its assertions and
  its fixture shape and now drives the launcher. `tools/host-patch.sh` is the other interpreted
  tool `sjel update apply` runs, delegated to because it owns brew, uv and rustup, and it stays
  bash. Verified against the script on this Mac:
  the live run was identical — 345 installed packages, 5 crate lockfiles, the same 36 findings
  across 19 globally installed packages, exit 1 in both — as were `-h`, no argument and an unknown
  argument, after normalizing the timestamp, the per-run temporary directory's name, osv-scanner's
  own inode and elapsed counters, and the table width osv-scanner derives from that name. Three
  differences are deliberate and named in `tools/sjel-cli/README.md`: the second pass's CycloneDX
  SBOM is built in the binary rather than by `jq` (so a missing `jq` can no longer read as an
  unscanned surface), JSON object keys are sorted where `jq` wrote them in filter order, and `-h`
  prints the whole header comment with its `#` markers stripped where the script printed only its
  first nineteen lines and cut off mid-sentence. `toolchain.toml`'s `[jq]` row stays — its `why` no
  longer claims the audit reads JSON, which is the only thing that changed about it.
- 2026-10-04 · `tools/self` is Rust, and `tools/self.ts` with `tools/lib/self-model.ts` is deleted.
  It moved for a measured reason: `tools/doctor` runs `tools/self check` on every invocation, so a
  bun start-up sat inside a Rust tool's path, and agents run `self explain` and `self coupling` at
  orientation. `src/self_model/mod.rs` holds the verb surface and all I/O; `src/self_model/model.rs`
  holds the pure half — the five-way path classification, the graph rollup, and the `#[path]` and
  Cargo-path coupling readers — with the cases `tools/self.test.ts` held as Rust unit tests.
  `tools/self.test.sh` stays and now drives the launcher. Verified against the TypeScript on this
  checkout: `generate` produced a byte-identical artifact except one deliberate line, and
  `status`, `status --json`, `explain` (text and `--json`, known and unknown unit), `coupling`
  (text and `--json`), `check` on a current artifact and on a stale one including the `diff -u`
  rendering, and `-h` were identical in stdout, stderr and exit code. Three differences are
  deliberate and named in `tools/sjel-cli/README.md`: the artifact's `generator` reads `tools/self`
  where it read `tools/self.ts` (self.json was regenerated in the same change, and that one line is
  the whole artifact diff), the help and status header now say `Sjel's self-model` where the
  TypeScript still said `Axon's` (the repository-wide rename, ISA F3), and the rollup no longer
  builds the `node id -> unit` map nothing read.
- 2026-10-04 · The four `packs-*` adapters are Rust, and with them `tools/lib/pack-deploy.ts`,
  `tools/lib/harness-registry.ts` and `tools/pack-drift-hook.ts` are deleted. This is the port the
  earlier ones were building toward: `tools/harnesses` had moved `pack-deploy.ts`'s mutation half
  into `src/harnesses/mutate.rs` on 2026-10-04, so the four adapters were the only thing keeping
  1,320 lines of TypeScript alive as a second implementation of one ledger — the duplication the
  crate exists to remove. `src/packs.rs` is their verb surface, four dispatchers over the engine
  rather than a flag table, because the four CLIs differ in load-bearing ways: claude heads a
  multi-pack write with the Pack name and codex does not, codex alone can `migrate-generated`, and
  pi is a settings registry rather than a copy. New engine code is only what those verbs needed:
  `migrate_generated_artifacts` (the one `pack-deploy.ts` function with no Rust half) and
  `profile_active_packs`, plus pi's settings-ledger `status`/`deploy`/`remove` in
  `pi_settings.rs`. The three non-adapter readers moved in the same change so that no ledger has
  two readers: the drift hook became `src/pack_hook.rs` and runs in-process on every session start
  and file change, `harness-registry.ts` was deleted with its last consumer keeping pi's marker
  inline, and `tools/generate-marketplace.ts` reads a new `sjel packs list` verb — as does
  `sjel search`, which used to shell `packs-opencode list` through bun. Verified against the
  TypeScript on two identical scratch roots before deletion: 114 comparisons across every verb of
  all four adapters, `-h` and unknown-verb paths, and `migrate-generated` in its refusal, positive
  and already-migrated paths, all identical in stdout, stderr and exit code after normalizing the
  scratch path. The deployed trees were identical except one file and the ledgers semantically
  identical except the digests it moves. Three differences are deliberate and named in
  `tools/sjel-cli/README.md`: the pi agent file's provenance comment now reads `Generated by Sjel
  tools/sjel-cli`, JSON object keys are sorted, and the migration's messages follow sorted ledger
  keys. `sjel pack <verb> <harness>` reaches the same four launchers it always did, and
  `tools/packs.sh` is still the one bash shim mapping `link`/`unlink` onto them.
- 2026-10-04 · Sjel's MCP server is Rust. `tools/sjel-mcp.ts` and its test are deleted, and the
  `sjel-mcp` crate now holds both halves of ISC-40: `src/server.rs` is the stdio JSON-RPC loop,
  the gate-backed tool list and the approval polling, and `src/tools.rs` is the pure half that
  decides which tools a capability offers under the owner's mode, what an MCP-legal tool name is
  and what URL a call becomes. The crate's own README had recorded this move as pending — "the
  server migrates into this crate when it is next touched" — and it moved for two reasons: an
  agent talks to this process for a whole session, so bun was in the runtime rather than only the
  build, and the registration half beside it already spawned the server and spoke MCP to it. It
  reuses one new thing and no new code: `libs/sjel-http`'s blocking client supplies the timeout and
  user-agent, `tools/capability-auth --agent` supplies the token, and `tools/capability.sh
  registry` supplies the ports — the same launcher the other tools call. One thread per request,
  as the TypeScript's un-awaited `handle()` was, and the read loop joins them, so a `tools/list`
  whose stdin has already closed still answers. Verified against the TypeScript before deletion on
  this machine, one request stream through both — `initialize`, `tools/list`, `ping`, a
  notification, an unknown method, a malformed line, a live `tools/call` and an unknown name —
  with the same 92 tools and every reply byte-identical after normalizing the pseudonym tokens.
  The tokens differed between the runs and are meant to: `X-Sjel-Agent-Session` is one random id
  per process, so the same value returns the same token inside a conversation and a different one
  across processes. Three differences are deliberate and named in `tools/sjel-mcp/README.md`: the
  tool list is ordered by capability where the TypeScript took `readdir` order, JSON object keys
  are sorted where the TypeScript wrote insertion order, and a `202` with no approval id answers
  with its body where the TypeScript read the body twice and threw. Held by 20 cases in
  `cargo test -p sjel-mcp`, and `sjel mcp`, `sjel mcp register` and `sjel mcp unregister` all exec
  the one binary now.
- 2026-10-04 · The browser session is a shared identity in the inbound gate, and the soundscape
  panel works. It was a table in the shared store that only the shell could read, so a capability
  serving its own panel — whose browser loads `:8088` directly and carries no token — answered
  `401` to the panel and to every request it made (ISA ISC-45's last open item). It is now
  `HMAC-SHA256(deployment token, payload)` with a 30-day expiry (`libs/sjel-server/src/session.rs`),
  which `InboundAuth::resolve` installs as a `SessionVerifier` for every capability: no store to
  read, no proxy in the path, no second secret, and rotating the deployment token revokes every
  session. The cookie slides on use, because whichever capability the browser reached re-issues
  it — that is what keeps a sliding session stateless. `SESSION_OPEN_PATH` is exempt only where
  the shell calls `with_session_open()`, so a capability does not exempt a route it does not
  serve. `sjel-status/src/session.rs` keeps the ticket and the handlers and drops its table;
  `hmac` is a new upstream row, pairing with the workspace's existing `sha2`. Verified live: no
  cookie `401`, a session minted through the shell `200` on the panel, its API and the shell
  itself, a bogus cookie `401` on both, and a reused ticket `401`. The trade is stated rather
  than hidden: a single session cannot be revoked on its own, because a stolen token stays valid
  until it expires.
- 2026-10-04 · `tools/harnesses` is wholly Rust. The write verbs — `sync`, `use`, `promote`,
  `accept` — move to `tools/sjel-cli/src/harnesses/` with `tools/lib/pack-deploy.ts`'s mutation
  half (the state lock, the atomic install, the digest policy, the ledger writes, profiles) and
  pi's settings registry (`tools/packs-pi.ts`'s activation: `settings.json` skills/extensions/
  packages, the pi settings ledger, the flat agent channel). `tools/harnesses.ts` and its test
  are deleted; the two pure helpers it exported have Rust unit tests. The whole mutation engine
  and the pi registry are two engines, not one, which is why this is the largest port: `use`
  activates a profile on pi's registry, and splitting at the verb level would have left
  `installOne`/the lock/the ledger in both languages. Every verb was compared against the
  TypeScript on two identical scratch roots (a copy of two Packs, `profiles.toml` and `tools/`,
  every harness destination and state file pointed at the scratch tree): `sync` undeployed, per
  pack and `--all`; `use` on claude and on pi; `promote`; `accept` — identical stdout, stderr,
  exit codes, destination trees, rewritten `pack.toml` and ledgers. Two differences are
  deliberate. The JSON ledgers and `settings.json` write map keys sorted where TypeScript wrote
  insertion order (a ledger is read by name, and the comparison above is semantic for JSON). And
  the provenance comment in a materialized pi agent file now reads `Generated by Sjel
  tools/sjel-cli` where the TypeScript wrote `Generated by Axon tools/packs-pi` — the generator
  moved and the old name is retired, so a deployed copy is rewritten once and its recorded
  digest changes with it.
- 2026-10-04 · `tools/updates` is Rust, and `tools/updates.ts` and `tools/updates.test.ts` are
  deleted. The report half and the apply half moved together, because `apply` re-reads the report
  after its steps and splitting them would have left two readers of the same rows. The launcher
  keeps its path and execs `tools/sjel-cli/src/updates/`, so `sjel update`, `tools/audit` and the
  dashboard are unchanged, and the audit's `--json --offline --inventory` no longer needs bun.
  Compared against `updates.ts` at HEAD before it was deleted: every flag combination — `-h`,
  `--offline`, `--json`, `--json --offline --inventory`, the five refusal paths, the live report
  and `--json`, and the four `apply` plans refused on a non-TTY — with identical stdout, stderr
  and exit codes after normalizing `generatedAt` and the ages that tick between two runs. One
  difference was found and fixed rather than recorded: `serde_json`'s default map sorts keys, and
  npm's NESTED dependency order is not sorted, so the `inventory` array came out in a different
  order. An order-preserving map in `updates/parse.rs` restores npm's order without turning on
  `serde_json/preserve_order`, which would change `Map` for every crate in the build. The parsers
  and gatherers are 137 `cargo test -p sjel-cli` cases, the fixtures the TypeScript suite used.
- 2026-10-03 · The audit now reaches this machine's own software, and the claims that said it
  already did are corrected. Nothing scanned what `sjel update` is uniquely responsible for
  moving: `tools/audit`'s `osv-scanner` read this repository's lockfiles, Dependabot read the
  same ones, and a globally installed crate or npm package was outside both — while
  `ARCHITECTURE.md` and `capabilities/host-patch/README.md` described the audit as running "over
  the machine it just patched". `tools/audit` gains a second `osv-scanner` pass over the whole
  installed inventory — a CycloneDX SBOM of the full npm tree plus each installed crate's own
  published `Cargo.lock` — and `sjel update apply` ends in `tools/audit` with the verdict on its
  receipt, so `apply --only cargo` and `--only npm` no longer install software nothing checks.
  ISC-59. Measured rather than assumed: osv-scanner 2.6.0's `directory` plugin extracts nothing
  from installed software (0 Extract calls on three different targets, including a valid
  `var/lib/dpkg/status`), so the SBOM is the mechanism that works. Also corrected in this change:
  CONTRIBUTING.md's coverage table said "Nothing asks" about a host package being behind, and
  had no row at all for this surface. The panel shows every verdict including `clean` — a silent
  clean would make a missing audit indistinguishable from a passing one — and colours a non-clean
  one in the summary line it already renders, so ISC-55's still-owed browser check now covers a
  verdict too.
- 2026-10-03 · Two add-ons off, and `apply` learned to remove and to re-resolve. The first
  honest audit run named Sjel's own harness trees, and neither turned out to be fixable by
  anything here: `@marckrenn/pi-sub-bar@1.5.0` peer-locks the deprecated `@mariozechner` scope
  and has no successor on npm (404 on `@earendil-works/pi-sub-bar`), and
  `claude-agent-sdk-pi@1.0.22` — the latest release — asks for
  `@earendil-works/pi-coding-agent: ^0.74.0` against an installed `1.0.0`, so npm nests an old
  copy. Both were already at their latest, so no upgrade was the fix; both names were unreferenced
  in this repository, the overlay, pi's `settings.json` and `mcp.json`, `~/.claude.json` and
  `~/.codex/config.toml`; so both came off. 399 packages and 11 advisories went with them (541
  installed → 439, 23 affected → 18). The other six unreferenced globals stayed, because
  `playwright/cli` and `bobshell` may be invoked by hand and no config would show it. ISC-60
  covers the two new flags: `--prune` removes only rows the report marks `removable`, which a
  pinned copy never is, and prints them before the `--yes` gate; `--re-resolve <crate,...>` drops
  `--locked` for the crates named and no others, so a crate whose published lockfile pins a
  flagged dependency can be reinstalled without giving up reproducibility everywhere. Neither
  was exercised live — nothing on this host is removable and no crate was stale — which ISC-60
  records rather than leaves implied.

  **The measurement is the point, and it is not comfortable.** 541 installed packages scanned:
  23 affected by 46 advisories, 3 Critical and 16 High, 36 fixable. The narrower first cut of
  this pass read only the top level — 13 npm packages and 4 crates — and reported this machine
  clean; the nested npm nodes and the crates' transitive trees are where the advisories were. So
  `tools/audit` exits 1 as of this change and `host-patch` with it, which its contract already
  means (`1 = the audit found something`, not `2 = a step failed`). Nothing this change ships
  silences that: an `osv-scanner.toml` exception is the repository's mechanism for a dated
  verdict, and whether these get fixed, excepted or scoped is the operator's next decision, not
  a default baked in here.
- 2026-10-01 · Main green again, the repository's dependencies moved, and this file cleaned.
  CI and Pages had been red since ISC-45: strict clippy on Linux refused three macOS-only imports
  in `libs/sjel-credentials` (b53f1f56), and the demo seeder got a 403 because a capability with
  no inbound token now fails closed (fddc43e2: `tools/demo-up` writes a throwaway
  `deployment.env`). That held every Dependabot pull request. Then four dependency moves. 137
  lockfile updates Dependabot does not propose forced xberg 1.3.0 (d5c22c3e, recorded in
  `upstreams.toml` [xberg]). thiserror 2 (ce8169f5) and html2text 0.17 (c86e2b7a, a code change
  and a test) closed #225 and #226. The dashboard runs TypeScript 7 through `--tsgo` beside 6.x,
  the setup soundscape already ran (5c8e1f99). ISA: ISC-27…30 moved to F3, ISC-55
  unticked (the browser check it owes is not done), ISC-29 and ISC-39 given their current
  state, and Decisions and Log put newest first. Seven Not-yet-specified entries were removed
  as fixed or superseded: the database-URL call sites (PRD Q45), the comms test (c473694b), the
  demo's missing areas (22793de3, eceddd96), the scheduled job's build PATH (c7b4ed2a), the names
  ISC-13 kept (now ISC-27…30), the installs past cooldown (Q77 retired the cooldown, and xberg is
  1.3.0), and the stale workflow worktrees (none remain).
- 2026-10-01 · F11 finished, and the rest of the machine with it. `tauri-cli` was the last
  upgrade and it was invisible: `cargo search` reports only the maximum version, so the released
  patch 2.12.1 sat hidden behind `3.0.0-alpha.4` and the tool called the crate current (ISC-58,
  now 2.12.1). The pre-existing audit finding was one package: `devalue` was pinned to exactly
  5.9.2 in `dashboard/package.json` and `capabilities/soundscape/ui/package.json`, carrying 14
  advisories (8 High, worst 8.2) against a fix in 5.9.3 — bumped to 5.9.3 in both, which
  satisfies the `^5.9.2` ranges svelte and kit ask for where 6.x would not. `tools/audit` now
  exits 0, so the host-patch receipt's `audit: finding` should clear on the next run. Two
  leftovers were removed rather than upgraded, neither deprecated and neither a version problem:
  npm's `uv@1.4.0` (an unrelated JS library with no bin and no dependents, colliding on the name
  with brew's uv) and `websockets` 16.0 in brew's python site-packages, which pip refuses to
  touch under PEP 668 and brew discards on the next python bump. That second one is why no `pip`
  class was added to `sjel update`: a class that can only hold leftovers in a directory brew
  wipes is a warning channel that stops being read. Everything else on the machine is already
  current — ollama 0.35.0 is the latest release, and pi and Claude Code self-update and sit at
  their latest. **`sjel update` reports nothing stale and exits 0.**
- 2026-10-01 · F11 continued, and the upgrade of everything else run through it. 11 stale npm
  globals were not 11 upgrades: `claude-agent-sdk-pi@1.0.22` and `@marckrenn/pi-sub-bar@1.5.0`
  dropped the `@mariozechner` lineage first (the latter only partly — it still bundles
  `@mariozechner/pi-coding-agent@0.73.1`, which upstream has not migrated), which released
  `@sinclair/typebox`. Done: pi-sub-bar 1.5.0, server-filesystem 2026.8.31, playwright/cli
  0.1.22, claude-agent-sdk-pi 1.0.22, defuddle 0.19.4, typebox 0.34.52, npm 12.2.0, pnpm 12.8.1,
  openjpeg 2.5.4_1, and both integration markers re-derived to today. The three leftovers were
  then REMOVED rather than upgraded (principal's call): `@mariozechner/pi-agent-core`, `pi-tui`
  and `pi-web-ui` at 0.52.12 were the pre-rename scope, which npm deprecates outright, and
  removing them took 64 packages with them. `sjel update` now reports nothing stale and exits 0 —
  the first time this machine has had a single answer to "is everything current". Two tool
  defects were found by using it (ISC-56) and one in `tools/agent-integrations.sh` that predates
  this work (ISC-57). The `apply --only brew` step reports as failed on this host because
  `tools/host-patch.sh` ends in `tools/audit`, whose pre-existing finding makes it exit 1 — the
  brew, uv and rustup steps themselves succeeded.
- 2026-10-01 · F11 built: `tools/updates` + `sjel update`, 53 unit tests, ISC-51…54 pass. Live
  probe on this host found four stale `cargo install`ed crates (macmon 0.7.0 → 0.8.2, at the
  `toolchain.toml` floor) and eleven stale global npm packages, and named the one trap worth
  encoding: `cargo search` returns pre-releases, so `tauri-cli` answered `3.0.0-alpha.4` over an
  installed `2.12.0` and ISC-54 exists because of it. The graphify and interceptor integration
  markers both read 2026-09-10 — 21 days — which is the drift ISC-51's row now shows.
  `apply --only cargo --yes` was then run on this host: bottom 0.14.7→0.14.9, macmon
  0.7.0→0.8.2, xberg-cli 1.0.14→1.3.0, 5m06s, `tauri-cli` left at 2.12.0 because only an alpha was
  newer. The Systems panel and its two routes landed the same day (ISC-55), verified by test
  rather than by eye — see that ISC for what could not be checked from an agent session.
- 2026-09-30 · ISC-31 landed: `sjel-status` serves `sjel storage report --json` at
  `GET /api/sjel-status/storage`, and the Systems page draws it — checked figure by figure
  against the tool, not against a 200. Reaching the page found a bug older than the feature: the
  rune probe at `dashboard/src/lib/models/decision-engine.svelte.ts:31` (481a2d5d, 2026-09-29)
  read `globalThis.$state`, which a browser defines as a getter that throws, so every route
  served SvelteKit's 500 page while 183 tests and `svelte-check` were green — nothing mounts a
  component. Fixed by making the probe catch. c68e7fb6 left ARCHITECTURE.md and its renamed
  inputs uncommitted; they land with this. Four repo gates stay red for neither reason:
  `capabilities/operator-profile/` is untracked (the F8 session, 18:12), which is what fails
  generator inputs, ARCHITECTURE freshness and `self.json`.
- 2026-09-30 · F9 built: keyed tokens and the agent view in `libs/pseudonymize`, the agent
  branch of the gate in `libs/sjel-server/src/agent.rs`, comms opted in, `sjel agent enroll`.
  ISC-41…43 pass as unit and HTTP tests. ISC-44 passes live: 364 rows, no address, no
  six-digit run, 13 Secret rows withheld.
- 2026-09-30 · F8: `sjel capability mail` reads comms triage with a c1 ceiling (ISC-36 for
  comms, ISC-39 partial); two classifier misses recorded under ISC-39.
- 2026-09-30 · F8 added with ISC-33…ISC-40, planning only. `tools/capability-auth` and the
  `sjel capability call` change (ISC-36, partial) landed uncommitted the same day.
- 2026-09-30 · F7 added with ISC-27…ISC-32; ISC-11 superseded and ISC-13 amended after the
  environment fallback was retired (9faf670a). A disk-pressure session reclaimed ~137 GB on
  this machine, which is how the blind spot F7 records was found.
- 2026-09-26 · F3 to F5 and five Not-yet-specified entries added from the session that named
  Sjel, switched the license and moved the product document into the README.
- 2026-08-19 · Scaffolded. Carries Axon issues #172, #174, #180 and the tracker
  retirement itself; #185 and #186 went to `Packs/travel/ISA.md`.
