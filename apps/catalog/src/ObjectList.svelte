<script lang="ts">
  import type { ObjectType, Row } from './bindings';
  let { object, rows, open }: { object: ObjectType; rows: Row[]; open: (row: Row) => void } = $props();
  const inactive = (row: Row) => object.inactive_property !== null && row[object.inactive_property] === true;
  const limit = 50;
  let page = $state(0);
  let visible = $derived(rows.slice(page * limit, (page + 1) * limit));
  $effect(() => { rows; page = 0; });
</script>
{#if rows.length === 0}<p>No {object.label.toLowerCase()} found.</p>{:else}
  <ul aria-label={object.label}>
    {#each visible as row (JSON.stringify(object.identity.map(key => row[key])))}
      <li class:inactive={inactive(row)}>
        <button onclick={() => open(row)}><span>{String(row[object.title])}</span><small>{String(row[object.classification.field])}{inactive(row) ? ` · ${object.inactive_property}` : ''}</small></button>
      </li>
    {/each}
  </ul>
  {#if rows.length > limit}<nav aria-label="List pages"><button disabled={page === 0} onclick={() => page--}>Previous</button><span>{page + 1} / {Math.ceil(rows.length / limit)}</span><button disabled={(page + 1) * limit >= rows.length} onclick={() => page++}>Next</button></nav>{/if}
{/if}
<style>
  ul { list-style: none; padding: 0; margin: 0; } li { border-bottom: 1px solid #cdd1d5; } li.inactive { opacity: .65; }
  li button { width: 100%; display: flex; gap: 1rem; justify-content: space-between; text-align: left; padding: .85rem .4rem; border: 0; background: transparent; }
  li button:hover { background: #e9edf0; } span { overflow-wrap: anywhere; } small { white-space: nowrap; color: #555e65; } nav { display: flex; align-items: center; gap: 1rem; margin-top: 1rem; }
</style>
