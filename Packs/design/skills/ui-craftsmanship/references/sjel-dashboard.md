# The rules in the Svelte dashboard

How each rule in `SKILL.md` is built in `dashboard/` today. This file is the part to rewrite if
the dashboard moves to GPUI. The rules in `SKILL.md` stay.

Verify every name below before you rely on it: `ls dashboard/src/lib/ui` and
`grep -oE '^\s*--[a-z0-9-]+' dashboard/src/app.css`.

## The kit

One shared kit lives in `dashboard/src/lib/ui`. It grows from real pages: add a primitive only
when a page needs it, and move other pages onto the kit one at a time (user decision,
2026-10-07). New UI starts from these.

| Primitive | Rule it carries | Notes |
|---|---|---|
| `ui/DataTable.svelte` | 1, 2, 3 | Columns with `align: "end"` get mono tabular figures. `inactive` dims a row. `actions` snippet is rung 3 (hover and focus, always shown with no hover). `onOpen` opens a row with click or Enter. `group` draws day or category headers. |
| `ui/Chip.svelte` | 1 | A value from a fixed set. `tone` is one of neutral, accent, success, warning, danger, muted. Map a value to a tone where the data is defined, not at the call site. |
| `ui/SidePeek.svelte` | 2 | Rung 4. A record's detail and editors beside the list. Not modal, Escape closes it, the next row is one click. |
| `ui/Property.svelte` | 1, 2 | One field of a record, edited in place. Replaces a separate edit form. |
| `ui/ViewSwitch.svelte` | 1 | Table, board and timeline over one dataset. |
| `ui/Collection.svelte` + `ui/collection.ts` | 1, 2 | A dataset as table, board or month calendar, with search, filter, sort (click a header) and group. Declare `Field`s; a `select` field filters, groups and splits the board; a `date` field turns on the calendar. Groups and board columns follow arrival order, so pass rows sorted the way they should read. With `id`, the state is in the URL under that prefix (`tx.sort=-amount`), so a view is a link. Without `id`, it is a plain `DataTable`. Use this, not a hand-built filter bar. |
| `ui/Section.svelte` | — | The one section header. Optional collapse, count and actions. |
| `lib/StateLine.svelte` | 3 | Loading, error and empty in one line. Empty renders nothing unless the caller passes `empty`. |
| `lib/tip.ts` (`use:tip`) | 3 | The tooltip. Shows on hover and keyboard focus, and becomes `aria-label` or `aria-description`. Never use `title`. |
| `lib/list-cursor.svelte.ts` | speed | J/K/Enter over a list, with real DOM focus. |
| `lib/modal.ts` | 2 | Focus trap for a true modal. Prefer `SidePeek` when the list must stay usable. |
| `.popover` in `app.css` | 2 | Rides the native `popover` attribute: top layer, light dismiss, Escape. |
| `lib/Icon.svelte` | tells | The only icon source. No emoji as icons. |

The layers of disclosure, from widest to narrowest:

1. Home's ranked list: what needs the reader now.
2. A context at `/context?day=`, `?trip=` or `?person=` (`lib/context/context.ts`): one
   Collection of every joined record (events, plan items, spend, trips), grouped by day.
   "Today" in the nav opens the day. A source that does not answer is named on the page.
3. The row, then a side peek or the inspector. The inspector links up to the context
   ("Everything around it") and down to the record page.
4. The record's own page at `/record?id=<typed id>` (`lib/inspector/record.ts`).

Rule 4 (show the join) is built here: a new kind of record joins a context by adding a row
builder and a source in `context.ts`, not by adding a page. The inspector links to the page with
"Open as page". After a reload, only ids that `expand` reads again (`cal:entry`, `trip:plan`)
resolve. Other ids show an error line that says so.

The global inspector (`lib/inspector/inspector.svelte`) is a second rung-4 surface. Use it when
the record has a linkable id that other capabilities can answer for (for example
`transactionItem` in `lib/inspector/connections`). Use `SidePeek` when the panel edits the
record or holds page-local detail.

## Tokens

Components read tokens from `dashboard/src/app.css`. Do not hardcode a colour, a size, a radius
or a duration. A `var(--x, #fallback)` that names an undeclared token is a defect: it shows the
fallback in every theme.

| Concern | Tokens |
|---|---|
| Text colour | `--text-primary`, `--text-secondary`, `--text-tertiary` |
| Tone | `--accent`, `--success`, `--warning` / `--warning-ink`, `--danger`, each with a `-soft` background |
| Surfaces | `--page-bg`, `--surface`, `--card-bg`, `--card-border`, `--rule`, `--nav-hover` |
| Type | `--font-sans`, `--font-mono`, `--text-2xs` to `--text-2xl`, `--leading-*` |
| Space | `--space-1` to `--space-8` |
| Radius | `--radius-sm`, `--radius-md`, `--radius-lg`, `--radius-full` (chips and pills only) |
| Motion | `--motion-fast`, `--motion-base`, `--motion-slow`, `--ease-out` |
| Focus | `--focus-ring` |

## Tests that enforce the rules

Run them with `bun test tools/dashboard-*.test.ts` from the repository root.

| Test | What it holds |
|---|---|
| `tools/dashboard-disclosure.test.ts` | `use:tip`, not `title`. Motion reads the tokens and never uses `transition: all`. Popovers stay native. |
| `tools/dashboard-row-disclosure.test.ts` | A row's reason is disclosed, not deleted. Hover, focus and tap all open it. A spent row is dimmed. |
| `tools/dashboard-tokens.test.ts` | Every custom property a component reads is declared. Every primitive in `app.css` has a consumer. |
| `tools/dashboard-markup.test.ts` | `<th>` has `scope`. A form button states its `type`. Inputs have a name, not only a placeholder. |
| `tools/dashboard-contrast.test.ts` | Text and tone tokens meet contrast on the surfaces they sit on. |
| `tools/dashboard-type-scale.test.ts` | Font sizes come from the type scale. |

## Known gaps

Measured 2026-10-08. Re-measure before quoting.

- Stale data has no shared primitive. Each page decides how to show the age of its data.
- Every data table uses `DataTable` or `Collection` since 2026-10-08. The `<table>`s left are
  on purpose: markdown and chart tables (`feed/`), the conflict diff in `SyncStatus` (its
  values must wrap), and interior's key/value readouts and sun heatmap.
