<script lang="ts">
  /**
   * What an item touches in other capabilities, as `connections.ts` answers it: references
   * first, then inferences, marked as such (libs/links/ISA.md D2). Shared by the inspector and
   * the people page, so a connection reads the same wherever it appears.
   *
   * A group renders only when it has rows or an error. An empty reference group means the
   * capability answered and nothing references the item; that is not worth a line.
   */
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";
  import type { InspectableItem } from "./inspector.svelte";
  import type { ConnectionGroup } from "./connections";

  let {
    groups,
    onfollow,
  }: { groups: ConnectionGroup[] | null; onfollow: (item: InspectableItem) => void } = $props();
</script>

{#if groups === null}
  <p class="state">Reading what this touches…</p>
{:else}
  {#each groups as g (g.capability + g.label)}
    {#if g.error || g.items.length > 0}
      <section class="group" class:coincidence={g.basis === "coincidence"} aria-label={g.label}>
        <span class="kicker">
          <Icon name={g.icon} size={11} />
          {g.label}
          {#if g.basis === "coincidence"}
            <span class="basis" use:tip={"Matched by text, dates or place. Holds no reference to this item."}>inferred</span>
          {/if}
          {#if !g.error}<span class="mono count">{g.items.length}</span>{/if}
        </span>
        {#if g.error}
          <p class="state">{g.error}</p>
        {:else}
          <ul>
            {#each g.items as r (r.key)}
              <li>
                <button type="button" class="row" onclick={() => onfollow(r.item)}>
                  <span class="title">{r.title}</span>
                  <span class="meta mono">{r.meta}</span>
                </button>
              </li>
            {/each}
          </ul>
          {#if g.unlinkable}
            <p class="state">
              {g.unlinkable} more {g.unlinkable === 1 ? "row references" : "rows reference"} this
              without an id that can be opened.
            </p>
          {/if}
        {/if}
      </section>
    {/if}
  {/each}
{/if}

<style>
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    background: var(--card-bg);
    border-radius: var(--radius-md);
    border: 1px solid var(--card-border);
  }

  /* An inferred group reads quieter than a referenced one: dashed edge, no fill. */
  .group.coincidence {
    background: transparent;
    border-style: dashed;
  }

  .kicker {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-tertiary);
  }

  .basis {
    font-weight: 500;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-tertiary);
    cursor: help;
  }

  .count {
    margin-left: auto;
    color: var(--text-secondary);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-1) var(--space-2);
    margin: 0 calc(-1 * var(--space-2));
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    font: inherit;
    font-size: var(--text-xs);
    text-align: left;
    cursor: pointer;
    transition: background-color var(--motion-fast) var(--ease-out);
  }

  .row:hover,
  .row:focus-visible {
    background: var(--surface);
  }

  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-primary);
    font-weight: 500;
  }

  .meta {
    flex-shrink: 0;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .state {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }
</style>
