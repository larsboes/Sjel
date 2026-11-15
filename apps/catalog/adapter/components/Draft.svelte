<script lang="ts">
  import { getStateContext, type ComponentRenderProps } from '@json-render/svelte';
  let { emit }: ComponentRenderProps = $props();
  const state = getStateContext();
  let note = $derived(String((state.state.draft as Record<string, unknown>).note ?? ''));
</script>
<section>
  <label for="proposal-note">Proposal note</label>
  <textarea id="proposal-note" value={note} oninput={event => state.set('/draft/note', event.currentTarget.value)} maxlength="2000"></textarea>
  <button onclick={() => emit('submit')} disabled={!note.trim()}>Prepare proposal</button>
</section>
<style>section { display: grid; gap: .5rem; } textarea { min-height: 5rem; font: inherit; } button { justify-self: start; }</style>
