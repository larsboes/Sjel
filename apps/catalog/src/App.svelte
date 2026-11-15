<script lang="ts">
  import { onMount } from 'svelte';
  import ObjectList from './ObjectList.svelte';
  import ObjectDetail from './ObjectDetail.svelte';
  import { registry, resolveBinding, linkedBinding, linkedRows, readRows, type Binding, type Row, type ObjectType, type Link } from './bindings';
  const root = registry.objects[0];
  const initial: Binding = { node: 'root', object: root.id, read: root.reads[0].id, parameters: {} };
  let binding = $state(initial);
  let object = $state<ObjectType>(root);
  let rows = $state<Row[]>([]);
  let selected = $state<Row | null>(null);
  let loading = $state(false);
  let error = $state('');
  let history = $state<Binding[]>([]);
  let controller: AbortController | undefined;
  let filter: { link: Link; row: Row } | undefined;
  let filters: (typeof filter)[] = [];

  async function load(next: Binding) {
    controller?.abort();
    const current = new AbortController(); controller = current;
    binding = next; loading = true; error = ''; selected = null;
    try {
      const resolved = resolveBinding(next);
      const result = await readRows(next, current.signal);
      if (current.signal.aborted) return;
      object = resolved.object;
      rows = filter ? linkedRows(filter.link, filter.row, result) : result;
      if (!resolved.read.many) selected = rows[0];
    } catch (cause) { if (!current.signal.aborted) { rows = []; error = String(cause); } }
    finally { if (!current.signal.aborted) loading = false; }
  }
  function open(row: Row) {
    selected = row;
    const link = object.links[0];
    if (link) {
      history = [...history, binding]; filters = [...filters, filter];
      filter = { link, row }; void load(linkedBinding(object, link, row));
    }
  }
  function back() {
    const previous = history.at(-1);
    if (previous) { history = history.slice(0, -1); filter = filters.pop(); void load(previous); }
  }
  onMount(() => { void load(initial); return () => controller?.abort(); });
</script>
<svelte:head><title>{object.label} · Sjel</title></svelte:head>
<header><strong>Sjel</strong><span>Catalog</span></header>
<main>
  {#if history.length}<button onclick={back}>← Back</button>{/if}
  <div class="heading"><h1>{object.label}</h1><button onclick={() => load(binding)}>Refresh</button></div>
  {#if error}<div role="alert"><p>{error}</p><button onclick={() => load(binding)}>Retry</button></div>
  {:else if loading}<p role="status">Loading {object.label.toLowerCase()}…</p>
  {:else if selected}<ObjectDetail {object} row={selected} />
  {:else}<ObjectList {object} {rows} {open} />{/if}
</main>
<style>
  :global(body) { margin: 0; background: #f7f8fa; color: #20272c; font: 16px/1.5 system-ui, sans-serif; }
  :global(button) { font: inherit; color: inherit; cursor: pointer; border: 1px solid #aeb7be; border-radius: 4px; padding: .35rem .7rem; background: #fff; }
  :global(button:focus-visible) { outline: 3px solid #176caf; outline-offset: 3px; } :global(button:disabled) { opacity: .5; cursor: default; }
  header { display: flex; gap: 1.5rem; padding: 1rem max(1rem, calc((100vw - 60rem)/2)); border-bottom: 1px solid #cdd1d5; }
  main { max-width: 60rem; margin: 1.5rem auto; padding: 0 1rem; min-height: 24rem; } .heading { display: flex; align-items: center; justify-content: space-between; gap: 1rem; } h1 { font-size: 1.5rem; }
</style>
