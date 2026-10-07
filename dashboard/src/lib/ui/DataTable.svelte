<script lang="ts" module>
  import type { Snippet } from "svelte";

  export interface Column<T> {
    id: string;
    label: string;
    /** CSS width: "5rem", "1fr" is not allowed in a table, leave it unset for the flexible one. */
    width?: string;
    /** "end" for numbers, so digits line up by place value. */
    align?: "start" | "end";
    cell: Snippet<[T]>;
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
    onOpen,
    inactive,
    selected,
    actions,
    empty = "Nothing here yet.",
  }: {
    rows: T[];
    columns: Column<T>[];
    key: (row: T) => string;
    /** Rows arrive sorted; a header row is drawn wherever this value changes. */
    group?: (row: T) => string;
    onOpen?: (row: T) => void;
    inactive?: (row: T) => boolean;
    selected?: string | null;
    actions?: Snippet<[T]>;
    empty?: string;
  } = $props();

  const span = $derived(columns.length + (actions ? 1 : 0));
</script>

{#if rows.length === 0}
  <p class="empty">{empty}</p>
{:else}
  <div class="wrap">
    <table>
      <colgroup>
        {#each columns as column (column.id)}<col style:width={column.width} />{/each}
        {#if actions}<col style:width="2.5rem" />{/if}
      </colgroup>
      <thead>
        <tr>
          {#each columns as column (column.id)}
            <th class:end={column.align === "end"} scope="col">{column.label}</th>
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
            {#each columns as column (column.id)}
              <td class:end={column.align === "end"}>{@render column.cell(row)}</td>
            {/each}
            {#if actions}<td class="row-actions">{@render actions(row)}</td>{/if}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: var(--text-xs);
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

  .end {
    text-align: right;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
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

  tr.openable:hover,
  tr.selected {
    background: var(--nav-hover);
  }

  tr:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: -2px;
  }

  tr.inactive td {
    color: var(--text-tertiary);
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
