<script lang="ts">
  import type { ObjectType, Row } from './bindings';
  let { object, row }: { object: ObjectType; row: Row } = $props();
</script>
<h2>{String(row[object.title])}</h2>
<dl>
  {#each Object.keys(object.schema.properties) as property}
    {#if property in row}<div><dt>{property.replaceAll('_', ' ')}</dt><dd>{row[property] === null ? '—' : String(row[property])}</dd></div>{/if}
  {/each}
</dl>
{#if object.actions.length}<p class="muted">Changes require a proposal through capability policy. This view is read only.</p>{/if}
<style>
  dl { margin: 0; } dl div { display: grid; grid-template-columns: minmax(7rem, 1fr) 3fr; gap: 1rem; border-bottom: 1px solid #cdd1d5; padding: .7rem 0; }
  dt { color: #555e65; } dd { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; } .muted { color: #555e65; }
  @media(max-width: 600px) { dl div { grid-template-columns: 1fr; gap: .3rem; } }
</style>
