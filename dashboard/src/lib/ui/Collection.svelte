<script lang="ts" generics="T">
  // One dataset as a table or a board, with search, filter, sort and group (2026-10-08).
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
  import { apply, optionsOf, readState, writeState, type CollectionState, type Field } from "./collection";

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
  // Seeded once from the URL; the toolbar writes `state` and the effect writes it back.
  let state = $state<CollectionState>(untrack(() => (id ? readState(page.url.searchParams, id, defaults) : defaults)));

  $effect(() => {
    if (!id) return;
    const url = new URL(page.url);
    writeState(url.searchParams, id, $state.snapshot(state), defaults);
    if (url.search !== page.url.search) replaceState(url, page.state);
  });

  const selects = $derived(fields.filter((f) => f.kind === "select"));
  const shown = $derived(id ? apply(rows, fields, state) : rows);
  const groupField = $derived(fields.find((f) => f.id === state.group));
  // The board splits by the grouped field, or the first select field when nothing is grouped.
  const boardField = $derived(groupField ?? selects[0]);
  const filterCount = $derived(Object.values(state.filter).reduce((n, v) => n + v.length, 0));
  const views = $derived([
    { id: "table", label: "Table", icon: "database" as const },
    ...(selects.length ? [{ id: "board", label: "Board", icon: "boxes" as const }] : []),
  ]);

  const text = (v: unknown) => (v == null ? "" : String(v));
  const label = (f: Field<T>, v: string) => (v === "" ? "None" : (f.display?.(v) ?? v));
  const byId = $derived(new Map(fields.map((f) => [f.id, f])));

  const columns: Column<T>[] = $derived(
    fields
      .filter((f) => f.column !== false)
      .map((f) => ({ id: f.id, label: f.label, width: f.width, align: f.align ?? (f.kind === "number" ? "end" : "start"), cell: f.cell ?? auto })),
  );

  function sortBy(field: string) {
    // Ascending, descending, then back to the order the rows arrived in.
    if (state.sort !== field) [state.sort, state.desc] = [field, false];
    else if (!state.desc) state.desc = true;
    else [state.sort, state.desc] = [null, false];
  }

  function toggle(field: string, value: string) {
    const current = state.filter[field] ?? [];
    const next = current.includes(value) ? current.filter((v) => v !== value) : [...current, value];
    const { [field]: _, ...rest } = state.filter;
    state.filter = next.length ? { ...rest, [field]: next } : rest;
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
    {#if views.length > 1}<ViewSwitch {views} bind:value={state.view} label="View" />{/if}
    <label class="search">
      <Icon name="search" size={12} />
      <input type="search" name="{id}-q" placeholder="Search" aria-label="Search" bind:value={state.q} />
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
                <input type="checkbox" name="{id}-f-{f.id}" checked={state.filter[f.id]?.includes(v) ?? false} onchange={() => toggle(f.id, v)} />
                {label(f, v)}
              </label>
            {/each}
          </fieldset>
        {/each}
        {#if filterCount}<button type="button" class="tool" onclick={() => (state.filter = {})}>Clear filters</button>{/if}
      </div>
      <label class="tool">
        Group
        <select name="{id}-group" bind:value={() => state.group ?? "", (v) => (state.group = v || null)}>
          <option value="">None</option>
          {#each selects as f (f.id)}<option value={f.id}>{f.label}</option>{/each}
        </select>
      </label>
    {/if}
    <span class="count mono" use:tip={"Shown of all rows"}>{shown.length}{#if shown.length !== rows.length}/{rows.length}{/if}</span>
  </div>
{/if}

{#if state.view === "board" && boardField}
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
    sort={id ? { id: state.sort, desc: state.desc } : undefined}
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

  .filters {
    display: grid;
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

  .card-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
</style>
