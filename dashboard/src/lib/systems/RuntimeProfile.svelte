<script lang="ts">
  import { onMount } from 'svelte';
  import Section from '$lib/ui/Section.svelte';
  import Chip from '$lib/ui/Chip.svelte';
  import StateLine from '$lib/StateLine.svelte';
  import { tip } from '$lib/tip';
  import { ApiError } from '$lib/api';
  import { runtime, runtimeView, RUNTIME_MODES, RUNTIME_CATEGORIES, POWER_LABELS, type RuntimeStatus, type RuntimeSelection, type RuntimeCategory, type RuntimeUpdate } from '$lib/runtime';

  let status = $state.raw<RuntimeStatus | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let loading = $state(false);
  const view = $derived(status ? runtimeView(status) : null);

  async function load() {
    if (loading || busy) return;
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

  async function save(update: RuntimeUpdate) {
    if (busy || loading || !status) return;
    busy = true;
    try {
      status = await runtime.update({ ...update, expected_revision: status.revision });
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      if (e instanceof ApiError && e.status === 409) {
        try {
          status = await runtime.status();
          error += ' Current policy reloaded. Review it and retry your change.';
        } catch (reloadError) {
          error += ` Could not reload current policy: ${reloadError instanceof Error ? reloadError.message : String(reloadError)}`;
        }
      }
    } finally {
      busy = false;
    }
  }

  async function allow(category: RuntimeCategory, enabled: boolean) {
    if (!status) return;
    const allowed = new Set(status.allow);
    if (enabled) allowed.add(category);
    else allowed.delete(category);
    await save({ allow: [...allowed] });
  }

  onMount(() => {
    void load();
    const timer = setInterval(load, 30_000);
    return () => clearInterval(timer);
  });
</script>

<div class="runtime">
  <Section title="Runtime profile">
    {#snippet actions()}
      <button type="button" class="btn" disabled={busy || loading} onclick={load}>Refresh</button>
    {/snippet}
    <StateLine state={error ? 'error' : !status ? 'loading' : 'ready'} message={error ?? 'Reading runtime profile…'} onRetry={load} />
    {#if status && view}
      <div class="summary" aria-live="polite">
        <Chip label={view.label} tone={view.onTheGo ? 'accent' : 'neutral'} />
        {#if view.afmOnly}<Chip label="AFM only" />{/if}
        <span>{view.source}{status.selection !== 'auto' ? ` · ${POWER_LABELS[status.power]}` : ''}{!status.power_fresh ? ' · power observation stale' : ''}</span>
      </div>
      {#if status.detail}<p class="hint">{status.detail}</p>{/if}
      {#if !status.configured}
        <p class="hint">Normal until you save a selection. Auto follows fresh power observations.</p>
      {/if}
      <form onsubmit={(e) => { e.preventDefault(); const data = new FormData(e.currentTarget); void save({ selection: data.get('selection') as RuntimeSelection }); }}>
        <label for="runtime-selection">Selection</label>
        <select id="runtime-selection" name="selection" class="input" value={status.selection} disabled={busy || loading}>
          {#each Object.entries(RUNTIME_MODES) as [mode, label] (mode)}
            <option value={mode}>{label}</option>
          {/each}
        </select>
        <button type="submit" class="btn" disabled={busy || loading}>{busy ? 'Saving…' : 'Save selection'}</button>
      </form>
      <p class="hint" id="runtime-exceptions">Persistent exceptions, applied only while On the go. Normal is unrestricted.</p>
      <ul aria-describedby="runtime-exceptions">
        {#each Object.entries(RUNTIME_CATEGORIES) as [category, label] (category)}
          <li>
            <label>
              <span>{label}</span>
              <input type="checkbox" role="switch" name={category} checked={status.allow.includes(category as RuntimeCategory)} disabled={busy || loading || !status.configured}
                use:tip={!status.configured ? 'Save a selection first' : busy || loading ? 'Runtime request in progress' : `Allow ${label.toLowerCase()} while On the go`}
                onchange={async (e) => {
                  const input = e.currentTarget;
                  await allow(category as RuntimeCategory, input.checked);
                  input.checked = status?.allow.includes(category as RuntimeCategory) ?? false;
                }} />
            </label>
          </li>
        {/each}
      </ul>
      <p class="hint">Active exceptions: {view.exceptions.length ? view.exceptions.join(', ') : 'none'}.</p>
    {/if}
  </Section>
</div>

<style>
  .runtime { margin: var(--space-5) 0; }
  .summary, form { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-3); font-size: var(--text-sm); }
  .summary { color: var(--text-secondary); }
  .hint { color: var(--text-tertiary); font-size: var(--text-xs); margin: var(--space-2) 0; }
  form { margin: var(--space-3) 0; }
  select { width: auto; }
  ul { list-style: none; padding: 0; margin: 0; display: grid; gap: var(--space-2); }
  li label { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); padding: var(--space-2) var(--space-3); border: 1px solid var(--card-border); border-radius: var(--radius-sm); font-size: var(--text-sm); }
  input { accent-color: var(--primary); }
</style>
