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

- [ ] DSH-1 — the menu bar panel shows a capability as down only when it is down. Falsifier: a
  capability shown down while its own `/health` answers 200. On 2026-10-08 the panel showed nine
  of 22 capabilities down, `dashboard` among them while the dashboard worked, so at least one
  health URL is wrong. Until this holds, the "needs you" dot stays on.
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

## Not yet specified

- **Home: a ranked list, or a view per context.** A view per trip, person or day would pull the
  areas together. That is the larger redesign. It waited for declared joins, which exist since
  2026-10-06, so it is the next candidate.
- **Home's full "now" in the menu bar.** Needs a server endpoint first, because Home is eleven kinds
  assembled in the browser (`dashboard/src/lib/home/registry.ts`).
- **A GPUI evaluation.** Decide what to measure before building anything. A candidate: the time from
  keypress to painted row on a 1,000-row table, against the Svelte `DataTable`. Not designed.
- **Import and export per capability, through one contract.** Each capability names its formats in
  (CSV, ICS, vCard, mbox) and out, and the shell gives them one place. Then every area is
  replaceable: Sjel holds the joins, and an external tool can hold the rows. Not designed.
- **Joins with nothing to return yet.** No ledger transaction carries a trip tag, so finance → trips
  answers empty. Layouts publish no join. Trips keeps its own place namespace (`obsidian-place:…`),
  so a place cannot list the trips that went there (`libs/links/ISA.md`, Out of scope).

## Log

- 2026-10-09 · Created from the vault note's "Next session" and "Direction" sections. Two items
  changed on the way. The note's eight hand-built tables are two now. "People are names, not ids"
  is dropped, because `libs/links` F5 (LNK-14…18) links names to `ent:` ids.
