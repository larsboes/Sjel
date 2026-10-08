<script lang="ts" generics="T">
  // One dataset as a table, a board or a month calendar, with search, filter, sort and group
  // (2026-10-08). The calendar appears when a field has `kind: "date"`.
  // The rows and their order come from `collection.ts`; the state lives in the URL under
  // `id`, so a view can be bookmarked. Without `id` it is a plain table with no toolbar,
  // for previews such as "recent transactions".
  import type { Snippet } from "svelte";
  import { untrack } from "svelte";
  import { page } from "$app/state";
  import { replaceState } from "$app/navigation";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import Chip from "./Chip.svelte";
  import DataTable, { type Column } from "./DataTable.svelte";
  import ViewSwitch from "./ViewSwitch.svelte";
  import { apply, dayOf, monthGrid, optionsOf, readState, shiftMonth, writeState, type CollectionState, type Field } from "./collection";

  let {
    id,
    rows,
    fields,
    key,
    title,
    defaults: given = {},
    onOpen,
    inactive,
    selected,
    actions,
    empty = "Nothing here yet.",
  }: {
    /** URL prefix for this collection's state. Unset: no toolbar, no URL state. */
    id?: string;
    rows: T[];
    fields: Field<T>[];
    key: (row: T) => string;
    /** A board card's heading. */
    title: (row: T) => string;
    defaults?: Partial<CollectionState>;
    onOpen?: (row: T) => void;
    inactive?: (row: T) => boolean;
    selected?: string | null;
    actions?: Snippet<[T]>;
    empty?: string;
  } = $props();

  const defaults: CollectionState = $derived({ view: "table", q: "", sort: null, desc: false, group: null, filter: {}, ...given });
  // Seeded once from the URL; the toolbar writes `current` and the effect writes it back.
  let current = $state<CollectionState>(untrack(() => (id ? readState(page.url.searchParams, id, defaults) : defaults)));

  $effect(() => {
    if (!id) return;
    const url = new URL(page.url);
    writeState(url.searchParams, id, $state.snapshot(current), defaults);
    if (url.search !== page.url.search) replaceState(url, page.state);
  });

  const selects = $derived(fields.filter((f) => f.kind === "select"));
  const shown = $derived(id ? apply(rows, fields, current) : rows);
  const groupField = $derived(fields.find((f) => f.id === current.group));
  // The board splits by the grouped field, or the first select field when nothing is grouped.
  const boardField = $derived(groupField ?? selects[0]);
  const filterCount = $derived(Object.values(current.filter).reduce((n, v) => n + v.length, 0));
  const dateField = $derived(fields.find((f) => f.kind === "date"));
  const views = $derived([
    { id: "table", label: "Table", icon: "database" as const },
    ...(selects.length ? [{ id: "board", label: "Board", icon: "boxes" as const }] : []),
    ...(dateField ? [{ id: "calendar", label: "Calendar", icon: "calendar" as const }] : []),
  ]);

  // The calendar's month is page-local, not in the URL: it opens where the rows are.
  const today = new Date().toLocaleDateString("sv-SE");
  let month = $state<string | null>(null);
  let opened = $state<Set<string>>(new Set());
  const byDay = $derived.by(() => {
    const out = new Map<string, T[]>();
    if (!dateField) return out;
    for (const row of shown) {
      const d = dayOf(dateField.value(row));
      if (d) out.set(d, [...(out.get(d) ?? []), row]);
    }
    return out;
  });
  const undated = $derived(dateField ? shown.filter((r) => !dayOf(dateField.value(r))).length : 0);
  // This month when it holds rows, else the month of the first dated row.
  const shownMonth = $derived(
    month ?? ([...byDay.keys()].some((d) => d.startsWith(today.slice(0, 7))) ? today.slice(0, 7) : ([...byDay.keys()].sort()[0]?.slice(0, 7) ?? today.slice(0, 7))),
  );
  const monthLabel = $derived(new Date(`${shownMonth}-15T12:00:00`).toLocaleDateString(undefined, { month: "long", year: "numeric" }));
  const toneOf = (row: T) => {
    const f = boardField;
    const v = f ? text(f.value(row)) : "";
    return (f && v && f.tone?.(v)) || "neutral";
  };
  const PER_DAY = 3;

  const text = (v: unknown) => (v == null ? "" : String(v));
  // `display` may name the empty value itself ("No day"); otherwise it reads "None".
  const label = (f: Field<T>, v: string) => (f.display?.(v) ?? v) || "None";
  const byId = $derived(new Map(fields.map((f) => [f.id, f])));

  const columns: Column<T>[] = $derived(
    fields
      .filter((f) => f.column !== false)
      .map((f) => ({ id: f.id, label: f.label, width: f.width, align: f.align ?? (f.kind === "number" ? "end" : "start"), cell: f.cell ?? auto })),
  );

  function sortBy(field: string) {
    // Ascending, descending, then back to the order the rows arrived in.
    if (current.sort !== field) [current.sort, current.desc] = [field, false];
    else if (!current.desc) current.desc = true;
    else [current.sort, current.desc] = [null, false];
  }

  function toggle(field: string, value: string) {
    const kept = current.filter[field] ?? [];
    const next = kept.includes(value) ? kept.filter((v) => v !== value) : [...kept, value];
    const { [field]: _, ...rest } = current.filter;
    current.filter = next.length ? { ...rest, [field]: next } : rest;
  }
</script>

{#snippet auto(row: T, column: Column<T>)}
  {@const f = byId.get(column.id)}
  {#if f}
    {@const v = text(f.value(row))}
    {#if f.kind === "select" && v}<Chip label={label(f, v)} tone={f.tone?.(v)} />{:else}<span class:soft={f.kind !== "number"}>{v}</span>{/if}
  {/if}
{/snippet}

{#if id}
  <div class="toolbar">
    {#if views.length > 1}<ViewSwitch {views} bind:value={current.view} label="View" />{/if}
    <label class="search">
      <Icon name="search" size={12} />
      <input type="search" name="{id}-q" placeholder="Search" aria-label="Search" bind:value={current.q} />
    </label>
    {#if selects.length}
      <button type="button" class="tool" class:on={filterCount > 0} popovertarget="{id}-filter" style:anchor-name="--{id}-filter">
        Filter{#if filterCount}<span class="mono"> {filterCount}</span>{/if}
      </button>
      <div id="{id}-filter" class="popover filters" popover style:position-anchor="--{id}-filter">
        {#each selects as f (f.id)}
          <fieldset>
            <legend>{f.label}</legend>
            {#each optionsOf(rows, f) as v (v)}
              <label>
                <input type="checkbox" name="{id}-f-{f.id}" checked={current.filter[f.id]?.includes(v) ?? false} onchange={() => toggle(f.id, v)} />
                {label(f, v)}
              </label>
            {/each}
          </fieldset>
        {/each}
        {#if filterCount}<button type="button" class="tool" onclick={() => (current.filter = {})}>Clear filters</button>{/if}
      </div>
      <label class="tool">
        Group
        <select name="{id}-group" bind:value={() => current.group ?? "", (v) => (current.group = v || null)}>
          <option value="">None</option>
          {#each selects as f (f.id)}<option value={f.id}>{f.label}</option>{/each}
        </select>
      </label>
    {/if}
    <span class="count mono" use:tip={"Shown of all rows"}>{shown.length}{#if shown.length !== rows.length}/{rows.length}{/if}</span>
  </div>
{/if}

{#if current.view === "board" && boardField}
  {@const f = boardField}
  <div class="board">
    {#each optionsOf(shown, f) as v (v)}
      {@const cards = shown.filter((r) => text(f.value(r)) === v)}
      <section class="column" aria-label={label(f, v)}>
        <header><Chip label={label(f, v)} tone={f.tone?.(v)} /><span class="mono soft">{cards.length}</span></header>
        {#each cards as row (key(row))}
          <button type="button" class="card" class:selected={selected === key(row)} class:inactive={inactive?.(row)} onclick={() => onOpen?.(row)}>
            <span class="card-title">{title(row)}</span>
            <span class="card-meta">
              {#each selects.filter((s) => s.id !== f.id).slice(0, 2) as s (s.id)}
                {@const sv = text(s.value(row))}
                {#if sv}<Chip label={label(s, sv)} tone={s.tone?.(sv)} />{/if}
              {/each}
            </span>
          </button>
        {/each}
      </section>
    {:else}
      <p class="soft">{empty}</p>
    {/each}
  </div>
{:else if current.view === "calendar" && dateField}
  <div class="month-bar">
    <button type="button" class="tool" onclick={() => (month = shiftMonth(shownMonth, -1))} use:tip={"Previous month"}><Icon name="arrow-left" size={12} /></button>
    <strong>{monthLabel}</strong>
    <button type="button" class="tool" onclick={() => (month = shiftMonth(shownMonth, 1))} use:tip={"Next month"}><Icon name="arrow-right" size={12} /></button>
    {#if shownMonth !== today.slice(0, 7)}<button type="button" class="tool" onclick={() => (month = today.slice(0, 7))}>This month</button>{/if}
    {#if undated}<span class="soft">{undated} without a date</span>{/if}
  </div>
  <div class="month" role="grid" aria-label={monthLabel}>
    {#each ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as wd (wd)}<span class="wd" role="columnheader">{wd}</span>{/each}
    {#each monthGrid(shownMonth) as d (d)}
      {@const dayRows = byDay.get(d) ?? []}
      {@const all = opened.has(d)}
      <div class="cell" class:outside={!d.startsWith(shownMonth)} class:today={d === today} role="gridcell" aria-label={d}>
        <span class="date mono">{Number(d.slice(8))}</span>
        {#each all ? dayRows : dayRows.slice(0, PER_DAY) as row (key(row))}
          <button type="button" class="entry tone-{toneOf(row)}" class:selected={selected === key(row)} class:inactive={inactive?.(row)} onclick={() => onOpen?.(row)} use:tip={title(row)}>
            {title(row)}
          </button>
        {/each}
        {#if dayRows.length > PER_DAY}
          <button type="button" class="more" onclick={() => (opened = new Set(all ? [...opened].filter((x) => x !== d) : [...opened, d]))}>
            {all ? "Fewer" : `+${dayRows.length - PER_DAY} more`}
          </button>
        {/if}
      </div>
    {/each}
  </div>
{:else}
  <DataTable
    rows={shown}
    {columns}
    {key}
    group={groupField ? (r) => label(groupField, text(groupField.value(r))) : undefined}
    {onOpen}
    {inactive}
    {selected}
    {actions}
    sort={id ? { id: current.sort, desc: current.desc } : undefined}
    onSort={id ? sortBy : undefined}
    empty={rows.length && !shown.length ? "No rows match this view." : empty}
  />
{/if}

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    font-size: var(--text-2xs);
  }

  .search {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    height: 1.6rem;
    padding: 0 0.45rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
  }

  .search:focus-within {
    outline: 2px solid var(--focus-ring);
  }

  .search input {
    width: 9rem;
    border: 0;
    background: none;
    outline: none;
    font: inherit;
    color: var(--text-primary);
  }

  .tool {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    height: 1.6rem;
    padding: 0 0.5rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    font: inherit;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .tool:hover,
  .tool:focus-within {
    border-color: var(--card-border);
    color: var(--text-primary);
  }

  .tool:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .tool.on {
    color: var(--accent);
  }

  .tool select {
    border: 0;
    background: none;
    font: inherit;
    color: var(--text-primary);
  }

  .count {
    margin-left: auto;
    color: var(--text-tertiary);
  }

  /* Only while open: a display on the closed sheet would override the UA's hidden popover. */
  .filters:popover-open {
    display: grid;
  }

  .filters {
    gap: var(--space-3);
    font-size: var(--text-xs);
  }

  fieldset {
    display: grid;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    margin-bottom: var(--space-1);
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  fieldset label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .soft {
    color: var(--text-tertiary);
  }

  .board {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(13rem, 1fr);
    gap: var(--space-2);
    overflow-x: auto;
    padding-bottom: var(--space-2);
  }

  .column {
    display: grid;
    align-content: start;
    gap: var(--space-1);
    min-width: 0;
  }

  .column header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: var(--space-1);
    border-bottom: 1px solid var(--rule);
    font-size: var(--text-2xs);
  }

  .card {
    display: grid;
    gap: 0.3rem;
    padding: 0.45rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    text-align: left;
    font: inherit;
    color: var(--text-primary);
    cursor: pointer;
  }

  .card:hover,
  .card.selected {
    border-color: var(--card-border-hover);
    background: var(--nav-hover);
  }

  .card:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .card.inactive {
    opacity: 0.55;
  }

  .card-title {
    font-size: var(--text-xs);
    line-height: var(--leading-tight);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .month-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    font-size: var(--text-xs);
  }

  .month-bar .soft {
    margin-left: auto;
    font-size: var(--text-2xs);
  }

  .month {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    border-top: 1px solid var(--rule);
    border-left: 1px solid var(--rule);
  }

  .wd {
    padding: var(--space-1) var(--space-2);
    border-right: 1px solid var(--rule);
    border-bottom: 1px solid var(--rule);
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .cell {
    display: grid;
    align-content: start;
    gap: 2px;
    min-height: 5.5rem;
    min-width: 0;
    padding: var(--space-1);
    border-right: 1px solid var(--rule);
    border-bottom: 1px solid var(--rule);
  }

  .cell.outside {
    background: var(--surface);
  }

  .cell.outside .date {
    color: var(--text-tertiary);
  }

  .date {
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .cell.today .date {
    color: var(--accent);
    font-weight: 600;
  }

  .entry,
  .more {
    padding: 1px var(--space-1);
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    text-align: left;
    font: inherit;
    font-size: var(--text-2xs);
    color: var(--text-primary);
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entry {
    border-left: 2px solid var(--text-tertiary);
  }

  .entry.tone-accent { border-left-color: var(--accent); }
  .entry.tone-success { border-left-color: var(--success); }
  .entry.tone-warning { border-left-color: var(--warning); }
  .entry.tone-danger { border-left-color: var(--danger); }
  .entry.tone-muted { border-left-color: var(--rule); }

  .entry:hover,
  .entry.selected,
  .more:hover {
    background: var(--nav-hover);
  }

  .entry:focus-visible,
  .more:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .entry.inactive {
    color: var(--text-tertiary);
  }

  .more {
    color: var(--text-tertiary);
  }

  .card-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
</style>
