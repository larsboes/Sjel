<script lang="ts">
  import { onMount } from 'svelte';
  import Chip from '$lib/ui/Chip.svelte';
  import { link } from '$lib/nav';
  import { tip } from '$lib/tip';
  import { runtime, runtimeView, type RuntimeStatus } from '$lib/runtime';

  let status = $state.raw<RuntimeStatus | null>(null);
  let error = $state<string | null>(null);
  let loading = false;
  const view = $derived(status ? runtimeView(status) : null);

  async function load() {
    if (loading) return;
    loading = true;
    try {
      status = await runtime.status();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }
  onMount(() => {
    void load();
    const timer = setInterval(load, 30_000);
    return () => clearInterval(timer);
  });
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
