---
name: ui-craftsmanship
description: Guides design, layout, styling and visual review of Sjel interfaces (the dashboard, capability UIs, components). Covers four rules — the data picks the form, the explicitness ladder for progressive disclosure, the state inventory of UI the reader does not see at first, and show the join across capabilities — plus a speed budget, anti-vibe-coding tells and mechanical sympathy. Use when designing or reviewing a page, table, panel or component, when moving a page onto the shared UI kit, when deciding where an action goes or whether a feature needs its own page, or when auditing an interface for generic AI or SaaS design tells. Do not use for backend Rust APIs, non-visual scripts or prose writing.
allowed-tools: Read, Write, Edit, Bash
---

# UI Craftsmanship

Sjel is a workstation that one person reads many times a day. A good view answers the reader's
question at a glance, keeps every other action one step away, and never makes the reader wait.

The rules below are written in terms of intent, not a framework, so they hold if the dashboard
moves from Svelte to GPUI (decision of 2026-10-08). The Svelte implementation of each rule —
kit primitives, tokens and the tests that enforce them — is in
[`references/sjel-dashboard.md`](references/sjel-dashboard.md). Read it before you edit
`dashboard/`.

Source for rules 1–3: Kole Jain, "The 3 dashboard UI flaws that give away you've never built
one" (https://www.youtube.com/watch?v=Ksx9C2-3yMo), adapted to Sjel. Rule 4 and the tip rule
come from the vault note `Projects/Sjel/Connected shell.md` (2026-10-06).

## Rule 1: the data picks the form

Before you choose a layout, name the shape of the data. Then use the form that shape asks for.

| The data is | Show it as | Not as |
|---|---|---|
| A value from a fixed set (status, category, kind) | A chip with a tone | Free text in a column |
| A number, an amount, a date, a count | Right-aligned, monospace, tabular figures | Left-aligned proportional text |
| Long free text in a dense row | One line, truncated, full text in a tip or the detail | A row that wraps to three lines |
| A row the reader cannot act on (cancelled, spent, zero) | Dimmed, still in place | Hidden, or styled like the live rows |
| Records ordered by time | A timeline or a list grouped by day | A table sorted by a timestamp column |
| Many records with the same fields | A table, with a board or timeline view of the same set | A grid of cards |
| One record with many fields | A property list in a side panel | A wide table with one row |
| A total that changes over time | A small chart next to the list it summarizes | Only the list |

Colour comes from the data. A tone says what a value means (settled, needs a decision, broken,
chosen, not active). Decorative colour is a defect. One urgent item in red reads instantly; ten
coloured items read as noise.

## Rule 2: the explicitness ladder

Every action has a place on a ladder from most to least visible. Put each action on the lowest
rung that its use still allows.

| Rung | Form | Use for |
|---|---|---|
| 1 | Labelled control, always visible | The one primary action of the view |
| 2 | Icon control, always visible, with a tip | Frequent actions where space is tight |
| 3 | Shown on hover and on keyboard focus | Secondary per-row actions (remove, copy, open externally) |
| 4 | Inside a popover, menu or side panel | Rare actions and editors (share, change a price, history) |
| 5 | Keyboard shortcut or command palette only | Power actions that already have a visible path elsewhere |

Rules for placing an action:

1. Rank by frequency first, then by cost of a mistake. A destructive action never sits on
   rung 1 or 2 next to the primary action.
2. A new feature goes into a side panel or popover on the page that owns its data. Give it a
   page of its own only when it has its own dataset and its own primary action.
3. Rung 3 must work without a mouse. An action that shows on hover also shows on focus, and is
   always visible on a device with no hover.
4. Hiding is sequencing, not deleting. A disclosed detail stays in the accessibility tree.
5. Lower rungs cost less to render. Content on rung 4 renders when it opens, not with the page.

## Rule 3: the state inventory

Most of a finished interface is the UI the reader does not see at first. Before a component is
done, check each state below. Write "not applicable" for a state only when you know why.

| State | Requirement |
|---|---|
| Hover | Rows and controls that react to a click show that they react |
| Focus | Every control is reachable by keyboard and shows a visible focus ring |
| Tip | Every icon-only control and every ambiguous label has a tip. The tip is also its accessible name. A tip never carries the only reason for something: the reason is also one click away |
| Empty | Say nothing when quiet is normal. Say what to do next when the reader expected data |
| Loading | Only for remote data. Reserve the final size so nothing shifts when data arrives |
| Error | Say what failed and offer retry where retry can work |
| Stale | Data older than its refresh interval shows its age |
| Disabled | A disabled control says why in its tip |
| Inactive | Spent or cancelled records are dimmed, not removed |
| Overflow | Long text truncates, and the full text is one hover or one click away |

## Rule 4: show the join

Sjel's value is the joins between its areas: the trip that explains the spend, the event that
explains the trip. Every view of one record lists what it touches in other capabilities, and
opens it in place (the inspector, `libs/links/ISA.md`). In review, a record view that lists
nothing outside its own capability gets a question.

## Speed budget

Speed is a feature of the design, not a later optimisation.

- A click or keypress shows a visible response in under 100 ms. Local data shows with no
  spinner.
- Every primary action has a keyboard path. Lists move with J/K and open with Enter.
- Motion animates only transform and opacity, and stays at or below the motion tokens
  (100–300 ms). Respect reduced motion.
- Prefer a native platform feature to a script: a native popover, a `<dialog>`, a `<details>`.
  In GPUI, prefer the framework's own element.
- Do not render what the reader cannot see (rule 2, item 5). Long lists render in pages or
  virtualize.

## Workflow

1. **Name the view's question.** A view answers one question ("what needs me today", "where did
   the money go"). The answer goes first. Everything else moves down the ladder.
2. **Shape the data** with rule 1. Use a kit primitive. Add a primitive to the kit only when a
   page needs one that does not exist.
3. **Place every action** with rule 2.
4. **Walk the state inventory** with rule 3, and **list the joins** with rule 4.
5. **Check the tells and the budget.** Read
   [`references/anti-slop-tells.md`](references/anti-slop-tells.md) and
   [`references/mechanical-sympathy-in-ui.md`](references/mechanical-sympathy-in-ui.md).
6. **Run the dashboard tests** listed in [`references/sjel-dashboard.md`](references/sjel-dashboard.md).

## Reference routing

| Topic | Reference |
|---|---|
| Svelte kit primitives, tokens, enforcing tests, how each rule is built today | [`references/sjel-dashboard.md`](references/sjel-dashboard.md) |
| The AI design tells, their code signatures and fixes | [`references/anti-slop-tells.md`](references/anti-slop-tells.md) |
| Typography, number alignment, spacing, density, semantic colour | [`references/visual-hierarchy-and-typography.md`](references/visual-hierarchy-and-typography.md) |
| Zero layout shift, compositor-only motion, contrast, reduced motion | [`references/mechanical-sympathy-in-ui.md`](references/mechanical-sympathy-in-ui.md) |
