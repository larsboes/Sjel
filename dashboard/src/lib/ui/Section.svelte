<script lang="ts">
  // One section header for every page (2026-10-07). Before this the dashboard had six class
  // names for the same idea: section-label, section-heading, section-head, section-kicker,
  // panel-header, card-header. New sections use this; old ones move over as they are touched.
  import type { Snippet } from "svelte";
  import Icon from "$lib/Icon.svelte";

  let {
    title,
    count,
    meta,
    collapsible = false,
    open = $bindable(true),
    actions,
    lead,
    children,
  }: {
    title: string;
    count?: number;
    /** One short line beside the title: a date range, a total. Not a sentence. */
    meta?: string;
    collapsible?: boolean;
    open?: boolean;
    /** Right-aligned controls: a view switch, an add button. */
    actions?: Snippet;
    /** Left of the title: a number, a status chip. */
    lead?: Snippet;
    children: Snippet;
  } = $props();
</script>

<section class="section" class:closed={collapsible && !open}>
  <header>
    {#if collapsible}
      <button type="button" class="toggle" aria-expanded={open} onclick={() => (open = !open)}>
        <span class="chev" class:open><Icon name="chevron" size={12} /></span>
        {@render lead?.()}
        <h3>{title}</h3>
      </button>
    {:else}
      {@render lead?.()}
      <h3>{title}</h3>
    {/if}
    {#if count !== undefined}<span class="count">{count}</span>{/if}
    {#if meta}<span class="meta">{meta}</span>{/if}
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </header>
  {#if !collapsible || open}
    <div class="body">{@render children()}</div>
  {/if}
</section>

<style>
  .section {
    display: grid;
    gap: var(--space-2);
    min-width: 0;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 1.75rem;
    border-bottom: 1px solid var(--rule);
    padding-bottom: var(--space-1);
  }

  h3 {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    border: 0;
    background: none;
    padding: 0;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .chev {
    display: inline-grid;
    color: var(--text-tertiary);
    transform: rotate(-90deg);
    transition: transform var(--motion-fast) var(--ease-out);
  }

  .chev.open {
    transform: rotate(0deg);
  }

  .toggle:hover .chev {
    color: var(--text-primary);
  }

  .count,
  .meta {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    white-space: nowrap;
  }

  .actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  @media (prefers-reduced-motion: reduce) {
    .chev {
      transition: none;
    }
  }
</style>
