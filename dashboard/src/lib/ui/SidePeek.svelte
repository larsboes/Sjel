<script lang="ts">
  // A record's detail in a panel on the right, with the list still visible and clickable
  // beside it (Notion's side peek). Not a modal: no backdrop, no focus trap, so opening the
  // next row is one click. Escape closes it. On a narrow screen it covers the page instead.
  import type { Snippet } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { tip } from "$lib/tip";

  let {
    title,
    eyebrow,
    onClose,
    footer,
    children,
  }: {
    title: string;
    eyebrow?: string;
    onClose: () => void;
    footer?: Snippet;
    children: Snippet;
  } = $props();

  let panel: HTMLElement | undefined = $state();

  $effect(() => {
    panel?.focus();
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape" && !(e.target as HTMLElement).closest("input, textarea, select")) onClose();
  }}
/>

<aside class="peek" bind:this={panel} tabindex="-1" aria-label={title}>
  <header>
    <div class="titles">
      {#if eyebrow}<span class="eyebrow">{eyebrow}</span>{/if}
      <h2>{title}</h2>
    </div>
    <button type="button" class="close" onclick={onClose} use:tip={"Close (Esc)"}>
      <Icon name="close" size={14} />
    </button>
  </header>
  <div class="body">{@render children()}</div>
  {#if footer}<footer>{@render footer()}</footer>{/if}
</aside>

<style>
  .peek {
    position: fixed;
    top: calc(var(--header-h, 3rem) + var(--space-2));
    right: var(--space-2);
    bottom: var(--space-2);
    width: min(26rem, calc(100vw - 2 * var(--space-2)));
    z-index: 40;
    display: grid;
    grid-template-rows: auto 1fr auto;
    background: var(--surface);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    box-shadow: var(--card-shadow-hover);
    outline: none;
    animation: enter var(--motion-base) var(--ease-out);
  }

  header {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-3) var(--space-2);
    border-bottom: 1px solid var(--rule);
  }

  .titles {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 0.15rem;
  }

  .eyebrow {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  h2 {
    margin: 0;
    font-size: var(--text-md);
    line-height: var(--leading-tight);
  }

  .close {
    display: grid;
    place-items: center;
    width: 1.75rem;
    height: 1.75rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .close:hover {
    background: var(--nav-hover);
    color: var(--text-primary);
  }

  .body {
    overflow-y: auto;
    padding: var(--space-3);
    display: grid;
    align-content: start;
    gap: var(--space-3);
  }

  footer {
    padding: var(--space-2) var(--space-3);
    border-top: 1px solid var(--rule);
    display: flex;
    gap: var(--space-3);
  }

  @keyframes enter {
    from {
      opacity: 0;
      transform: translateX(1rem);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .peek {
      animation: none;
    }
  }
</style>
