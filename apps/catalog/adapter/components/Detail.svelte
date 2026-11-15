<script lang="ts">
  import { getStateContext, type ComponentRenderProps } from '@json-render/svelte';
  import ObjectDetail from '../../src/ObjectDetail.svelte';
  import { selectedRow, type ReadSnapshot } from '../runtime';
  let { element }: ComponentRenderProps<{ binding: string }> = $props();
  const state = getStateContext();
  let snapshot = $derived((state.state.snapshot as Record<string, ReadSnapshot>)[element.props.binding]);
  let row = $derived(selectedRow(snapshot, (state.state.ui as Record<string, unknown>).selection, element.props.binding));
</script>
<section aria-label="Selected object" aria-live="polite">
  {#if row}<ObjectDetail object={snapshot.object} {row} />{:else}<p>Select an object to inspect its properties.</p>{/if}
</section>
