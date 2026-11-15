<script lang="ts">
  import { getStateContext, type ComponentRenderProps } from '@json-render/svelte';
  import ObjectList from '../../src/ObjectList.svelte';
  import type { ReadSnapshot } from '../runtime';
  let { element }: ComponentRenderProps<{ binding: string }> = $props();
  const state = getStateContext();
  let snapshot = $derived((state.state.snapshot as Record<string, ReadSnapshot>)[element.props.binding]);
</script>
<section>
  <h2>{snapshot.object.label}</h2>
  <ObjectList object={snapshot.object} rows={snapshot.rows} open={row => state.set('/ui/selection', {
    binding: element.props.binding, identity: snapshot.object.identity.map(key => row[key])
  })} />
</section>
