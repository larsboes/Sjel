<script lang="ts" module>
  import type { Snippet } from "svelte";

  export interface Column<T> {
    id: string;
    label: string;
    /** CSS width: "5rem", "1fr" is not allowed in a table, leave it unset for the flexible one. */
    width?: string;
    /** "end" for numbers, so digits line up by place value. */
    align?: "start" | "end";
    /**
     * Cells a reader reads rather than scans. Truncation is right for a label and wrong for a
     * value somebody has to compare, so this column wraps instead of ending in an ellipsis.
     */
    wrap?: boolean;
    /** Gets the column too, so one snippet can render several columns. */
    cell: Snippet<[T, Column<T>]>;
  }
</script>

<script lang="ts" generics="T">
  // The dashboard's table (2026-10-07). The data decides the form: callers render fixed sets
  // as Chips, numbers right-aligned in mono, long text truncated. Rows a reader cannot act on
  // are shaded (`inactive`). Secondary actions show on hover or keyboard focus, not always.
  // A row opens with a click or Enter; the caller decides what opening means (a side peek).
  let {
    rows,
    columns,
    key,
    group,
    caption,
    rowHeader,
    warn,
    onOpen,
    inactive,
    selected,
    actions,
    sort,
    onSort,
    empty = "Nothing here yet.",
  }: {
    rows: T[];
    columns: Column<T>[];
    key: (row: T) => string;
    /** Rows arrive sorted; a header row is drawn wherever this value changes. */
    group?: (row: T) => string;
    /** Names the table when the heading above it would not. */
    caption?: string;
    /** The first column names its row, so it renders as a `<th scope="row">`. */
    rowHeader?: boolean;
    /** Rows whose numbers a reader has to look at, in the warning ink. */
    warn?: (row: T) => boolean;
    onOpen?: (row: T) => void;
    inactive?: (row: T) => boolean;
    selected?: string | null;
    actions?: Snippet<[T]>;
    /** The sorted column; with `onSort`, headers become buttons that sort by their column. */
    sort?: { id: string | null; desc: boolean };
    onSort?: (id: string) => void;
    empty?: string;
  } = $props();

  const span = $derived(columns.length + (actions ? 1 : 0));
</script>

{#if rows.length === 0}
  <p class="empty">{empty}</p>
{:else}
  <div class="scroll">
    <table>
      {#if caption}<caption>{caption}</caption>{/if}
      <colgroup>
        {#each columns as column (column.id)}<col style:width={column.width} />{/each}
        {#if actions}<col style:width="2.5rem" />{/if}
      </colgroup>
      <thead>
        <tr>
          {#each columns as column (column.id)}
            {@const sorted = sort?.id === column.id}
            <th
              class:end={column.align === "end"}
              scope="col"
              aria-sort={sorted ? (sort?.desc ? "descending" : "ascending") : undefined}
            >
              {#if onSort}
                <button type="button" class="sort" class:sorted onclick={() => onSort(column.id)}>
                  {column.label}{#if sorted}<span aria-hidden="true">{sort?.desc ? " ↓" : " ↑"}</span>{/if}
                </button>
              {:else}{column.label}{/if}
            </th>
          {/each}
          {#if actions}<th scope="col"><span class="sr">Actions</span></th>{/if}
        </tr>
      </thead>
      <tbody>
        {#each rows as row, index (key(row))}
          {#if group && (index === 0 || group(rows[index - 1]) !== group(row))}
            <tr class="group"><th colspan={span} scope="rowgroup">{group(row)}</th></tr>
          {/if}
          <tr
            class:inactive={inactive?.(row)}
            class:warn={warn?.(row)}
            class:selected={selected === key(row)}
            class:openable={!!onOpen}
            tabindex={onOpen ? 0 : undefined}
            onclick={(e) => {
              if ((e.target as HTMLElement).closest("button, a, input, select")) return;
              onOpen?.(row);
            }}
            onkeydown={(e) => {
              if (e.key === "Enter" && e.target === e.currentTarget) onOpen?.(row);
            }}
          >
            {#each columns as column, position (column.id)}
              {#if rowHeader && position === 0}
                <th scope="row" class:wraps={column.wrap}>{@render column.cell(row, column)}</th>
              {:else}
                <td class:end={column.align === "end"} class:wraps={column.wrap}>
                  {@render column.cell(row, column)}
                </td>
              {/if}
            {/each}
            {#if actions}<td class="row-actions">{@render actions(row)}</td>{/if}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .scroll {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: var(--text-xs);
  }

  caption {
    padding-bottom: 0.35rem;
    text-align: left;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  th,
  td {
    padding: 0.3rem 0.5rem;
    text-align: left;
    vertical-align: middle;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    font-size: var(--text-2xs);
    font-weight: 500;
    color: var(--text-tertiary);
    border-bottom: 1px solid var(--rule);
  }

  .sort {
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  .sort:hover,
  .sort.sorted {
    color: var(--text-primary);
  }

  .sort:focus-visible {
    outline: 2px solid var(--focus-ring);
  }

  .end {
    text-align: right;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  /* The row's name, said in the row rather than by its place. */
  tbody th {
    font-weight: 400;
    color: var(--text-secondary);
  }

  .wraps {
    white-space: normal;
    vertical-align: top;
  }

  tbody tr:not(.group) {
    border-bottom: 1px solid var(--rule);
  }

  tr.group th {
    padding-top: var(--space-3);
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-secondary);
    border-bottom: 1px solid var(--rule);
  }

  tr.openable {
    cursor: pointer;
  }

  /* --nav-hover is a text colour; as a fill it turned the row black (light) or white (dark). */
  tr.openable:hover {
    background: var(--surface);
  }

  tr.selected {
    background: var(--accent-soft);
  }

  tr:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: -2px;
  }

  tr.inactive th,
  tr.inactive td {
    color: var(--text-tertiary);
  }

  tr.warn th,
  tr.warn td {
    color: var(--warning-ink);
  }

  .row-actions {
    text-align: right;
    opacity: 0;
    transition: opacity var(--motion-fast) var(--ease-out);
  }

  tr:hover .row-actions,
  tr:focus-within .row-actions {
    opacity: 1;
  }

  .empty {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }

  @media (hover: none) {
    .row-actions {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .row-actions {
      transition: none;
    }
  }
</style>
