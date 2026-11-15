<script lang="ts">
  import { JsonUIProvider, Renderer, type ComponentRegistry } from '@json-render/svelte';
  import type { StateStore } from '@json-render/core';
  import { parseView, rendererSpec } from '../document';
  import type { Binding } from '../../src/bindings';
  import Stack from './Stack.svelte';
  import Objects from './Objects.svelte';
  import Detail from './Detail.svelte';
  import Draft from './Draft.svelte';
  let { document, bindings, store, propose }: {
    document: string; bindings: Readonly<Record<string, Binding>>; store: StateStore;
    propose: (proposal: { kind: 'view-draft'; note: string }) => Promise<void>;
  } = $props();
  const registry: ComponentRegistry = { Stack, ObjectList: Objects, ObjectDetail: Detail, Draft };
  let admission = $derived.by(() => {
    try { return { spec: rendererSpec(parseView(document, bindings)), error: '' }; }
    catch (error) { return { spec: null, error: String(error) }; }
  });
  let proposalError = $state('');
  let handlers = $derived({ propose: async () => {
    proposalError = '';
    const note = String((store.get('/draft') as Record<string, unknown>).note ?? '').trim();
    if (!note || note.length > 2000) { proposalError = 'Proposal note must contain 1–2000 characters.'; return; }
    try { await propose({ kind: 'view-draft', note }); }
    catch (error) { proposalError = `Proposal was not prepared: ${String(error)}`; }
  } });
</script>
{#if admission.error}<p role="alert">{admission.error}</p>
{:else}
  <JsonUIProvider {store} {handlers}><Renderer spec={admission.spec} {registry} /></JsonUIProvider>
  {#if proposalError}<p role="alert">{proposalError}</p>{/if}
{/if}
