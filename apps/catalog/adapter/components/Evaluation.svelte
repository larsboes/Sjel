<script lang="ts">
  import Surface from './Surface.svelte';
  import { fixtureBindings, fixtureDocument, createFixtureState } from '../fixture-state';
  import type { ReadSnapshot } from '../runtime';
  const state = createFixtureState();
  let document = $state(JSON.stringify(fixtureDocument));
  let status = $state('Fictional data. Proposals do not write to a capability.');
  function refresh() {
    const snapshot = state.client.get('/snapshot/records') as ReadSnapshot;
    state.host.set('/snapshot/records', { ...snapshot, rows: snapshot.rows.map((row, i) => i ? row : { ...row, title: `${row.title} · refreshed` }) });
    status = 'Host snapshot refreshed. The draft is retained.';
  }
  function checkGuard() {
    try { state.client.set('/snapshot/records', {}); status = 'Error: snapshot write was accepted.'; }
    catch { status = 'Snapshot write refused.'; }
  }
</script>
<svelte:head><title>Sjel adaptive catalog evaluation</title></svelte:head>
<main>
  <h1>Adaptive catalog evaluation</h1>
  <p role="status">{status}</p>
  <nav aria-label="Evaluation checks">
    <button onclick={refresh}>Refresh host snapshot</button>
    <button onclick={() => { document = JSON.stringify(JSON.parse(JSON.stringify(fixtureDocument))); status = 'Saved view reloaded without a model.'; }}>Reload saved view</button>
    <button onclick={checkGuard}>Check snapshot guard</button>
  </nav>
  <Surface {document} bindings={fixtureBindings} store={state.client} propose={async proposal => {
    status = `Proposal prepared: ${proposal.note}. No capability write was made.`;
  }} />
</main>
<style>
  :global(body) { margin: 0; font: 16px/1.5 system-ui, sans-serif; color: #20272c; background: #f7f8fa; }
  main { max-width: 60rem; margin: 1.5rem auto; padding: 0 1rem; }
  nav { display: flex; flex-wrap: wrap; gap: .5rem; margin-bottom: 1.5rem; }
  :global(button) { font: inherit; padding: .4rem .7rem; cursor: pointer; }
  :global(button:focus-visible), :global(textarea:focus-visible) { outline: 3px solid #176caf; outline-offset: 3px; }
</style>
