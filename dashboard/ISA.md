---
project: dashboard
type: isa
phase: climbing
progress: 0
principal_stated_goal: "our connection is the outstanding point … so our system is interchangeable"
---

# ISA · dashboard

State of record for the shell: `dashboard/` and the menu bar app in `apps/mac`. Repo-wide items
stay in the root `ISA.md`. The join contract is `libs/links/ISA.md`. The UI rules are
`Packs/design/skills/ui-craftsmanship/SKILL.md`. Moved here on 2026-10-08 from the vault note
`Projects/Sjel/Connected shell.md`, which now points here.

## Problem

A mail app, a calendar and a ledger already exist. Sjel has a reason to exist only where it joins
them: the trip that explains the spend, the event that explains the trip. The dashboard grew one
page per capability, which is the shape of five separate apps that share a top bar.

The inspector follows joins since 2026-10-06 (`73dec7ee`, `dashboard/README.md`, "The inspector
follows connections"), and capabilities declare them since the same day (`libs/links/ISA.md`).
What is left is below.

## Claims

- [x] DSH-1 — the menu bar panel shows a capability as down only when it is down. Falsifier: a
  capability shown down while its own `/health` answers 200. On 2026-10-08 the panel showed nine
  of 22 capabilities down, `dashboard` among them while the dashboard worked, so at least one
  health URL is wrong. Until this holds, the "needs you" dot stays on. Measured 2026-10-09: no
  health URL was wrong. Every capability read down was stopped and not listening, and each was
  on-demand by its manifest. `dashboard` is the Vite dev server on 47117, not the shell that
  serves the built pages. The panel counted stopped-on-demand as down. Now
  `/api/sjel-status/health` carries `autostart` per capability, `NodeHealth.down` counts only
  `autostart` ones, and an on-demand row that is stopped shows a grey dot. After the restart:
  `ok: true`, no kept-running capability down, four on-demand stopped.
- [ ] DSH-2 — ⌃⌥Space opens Ask from any app, and the question arrives in the dashboard drawer.
  Falsifier: the drawer opens empty. Needs a `sjel-status` restart for the `next` change in
  `capabilities/sjel-status/src/session.rs`, not done on 2026-10-08 because another session had
  uncommitted work in that crate. Operator check.
- [ ] DSH-3 — `/finance` reads right with live data: the three `DataTable`s, the subscription
  `SidePeek`, and dimmed transfers. Falsifier: any of the three looks wrong with real rows. The demo
  recording has no finance views, so only the operator can check this.
- [ ] DSH-4 — a page header does not overlap its description at narrow widths. Falsifier: on
  `/finance` at a narrow width, "Money, as a system" runs into its description
  (`dashboard/src/lib/PageHeader.svelte`).
- [ ] DSH-5 — no page builds its own table. Markdown and chart tables are exempt. Falsifier:
  `grep -rl '<table' dashboard/src` lists a file other than `ui/DataTable.svelte`,
  `feed/ChartFigure.svelte` and `feed/MarkdownDocument.svelte`. On 2026-10-09 two remain:
  `SyncStatus.svelte` and `routes/interior/+page.svelte`.
- [ ] DSH-6 — every page that shows fetched data shows its age through one shared component.
  Falsifier: a page with fetched data and no age, or a page with its own age display (skill
  rule 3).
- [ ] DSH-7 — `Sjel.app` has an app icon, so its approval notifications show it. Falsifier: a
  notification with a blank icon.
- [x] DSH-8 — `GET /sjel-status/api/sjel-status/capabilities` answers in under 200 ms. Falsifier:
  any of five warm `curl` calls through the shell at `127.0.0.1:8082` takes 200 ms or more. On
  2026-10-09 the five took 1.0–1.4 s, and the shell fetched it four times during one load of
  `/context`, at 274–1,085 ms each. The 200 ms ceiling is a choice made that day, not a derived
  number. Every other call that page makes answered in 1–5 ms. Cause: the endpoint probed 21
  readiness URLs one after another, and vaultwarden over the tailnet took 0.3–1.0 s of it. Fixed
  the same day in `capabilities/sjel-status/src/status/health.rs` (`liveness`): concurrent probes,
  a 5 s cache refreshed in the background, and a reset on start and stop. The two other handlers
  with the same loop (`/health`, `/self`) use it too. Evidence: five warm calls at 1.3–1.6 ms
  after a restart, the first cold call at 0.37 s.

## Not yet specified

- **Home: a ranked list, or a view per context.** A view per trip, person or day would pull the
  areas together. That is the larger redesign. It waited for declared joins, which exist since
  2026-10-06, so it is the next candidate.
- **Home's full "now" in the menu bar.** Needs a server endpoint first, because Home is eleven kinds
  assembled in the browser (`dashboard/src/lib/home/registry.ts`).
- **A GPUI command palette.** The first GPUI surface, by the root `ISA.md` decision of
  2026-10-09. Rendering was measured first: a `/context` day switch settles in 26 ms in Chrome,
  so speed alone does not justify a second UI stack. The palette has to win on start time and
  resident memory. Measure both against a webview before building past
  `apps/gpui-prototype`, because neither number exists. Data comes over loopback HTTP, like
  the dashboard, with no direct database access. Open: whether it replaces the Swift ⌃⌥Space
  panel (`apps/mac/Sources/SjelMacApp/QuickAsk.swift`) or opens beside it. Also open: what it
  searches beyond Ask (records, pages, capability actions).
- **Database views, from Obsidian Bases and Notion.** One collection, many saved views: table,
  board, list and calendar, each with its own filter, sort, group and visible properties. A view
  sits inline in a page or fills one, and a record opens as a page with its properties on top.
  `lib/ui/Collection.svelte` already switches table and board (line 66). Saved views and
  per-view formulas do not exist. The planning owner (root `ISA.md`, "A planning owner") is
  the first collection that needs all of it.
- **A graph view.** The joins from `libs/links` and the vault's wikilinks, drawn as one graph. A
  vault of 1,123 Knowledge notes (vault `Domains/Knowledge.md`) is at the size where the renderer
  matters. Svelte with WebGL or GPUI is decided when the node count is measured. Not designed.
- **Import and export per capability, through one contract.** Each capability names its formats in
  (CSV, ICS, vCard, mbox) and out, and the shell gives them one place. Then every area is
  replaceable: Sjel holds the joins, and an external tool can hold the rows. Not designed.
- **Joins with nothing to return yet.** No ledger transaction carries a trip tag, so finance → trips
  answers empty. Layouts publish no join. Trips keeps its own place namespace (`obsidian-place:…`),
  so a place cannot list the trips that went there (`libs/links/ISA.md`, Out of scope).

## Log

- 2026-10-09 · DSH-8 added from a measurement of `/context`: the data calls take 1–5 ms, and the
  capabilities endpoint takes over a second. "A GPUI evaluation" became "A GPUI command palette"
  after the root decision. Database views and a graph view were added from the decision that
  moves projects, tasks and habits into Sjel.

- 2026-10-09 · Created from the vault note's "Next session" and "Direction" sections. Two items
  changed on the way. The note's eight hand-built tables are two now. "People are names, not ids"
  is dropped, because `libs/links` F5 (LNK-14…18) links names to `ent:` ids.
