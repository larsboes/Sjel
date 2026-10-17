<script lang="ts">
  import { onMount } from 'svelte';
  import Chip from '$lib/ui/Chip.svelte';
  import { link } from '$lib/nav';
  import { tip } from '$lib/tip';
  import { poll } from '$lib/ui/poll.svelte';
  import { runtime, runtimeView } from '$lib/runtime';

  // The sidebar badge reads the same profile /systems shows, at a slower beat: what
  // accelerates the machine changes when someone changes it.
  const polled = poll({ intervalMs: 30_000, fetch: () => runtime.status() });
  const status = $derived(polled.data);
  const error = $derived(polled.error);
  const view = $derived(status ? runtimeView(status) : null);

  onMount(() => polled.start());
</script>

<a href={link('/systems')} class="runtime-badge" use:tip={error ?? view?.summary ?? 'Reading runtime profile'}>
  {#if view}
    <Chip label="{view.label}{view.afmOnly ? ' · AFM only' : ''}" tone={error ? 'warning' : 'neutral'} />
    <span class="source">{view.source}{view.exceptions.length ? ` · ${view.exceptions.length} exception${view.exceptions.length === 1 ? '' : 's'}` : ''}{error ? ' · stale' : ''}</span>
  {:else}
    <Chip label={error ? 'Runtime unavailable' : 'Runtime…'} tone={error ? 'warning' : 'muted'} />
  {/if}
</a>

<style>
  .runtime-badge { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1); color: var(--text-secondary); text-decoration: none; }
  .source { font-size: var(--text-2xs); }
  .runtime-badge:focus-visible { outline: var(--focus-ring); outline-offset: var(--space-1); }
  .runtime-badge:hover { color: var(--text-primary); }
</style>
