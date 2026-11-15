import { render } from 'svelte/server';
import Surface from './components/Surface.svelte';
import { fixtureBindings, fixtureDocument, createFixtureState } from './fixture-state';
import type { ReadSnapshot } from './runtime';

/** Render the real upstream Svelte renderer with fictional classified host data. */
export function renderFixture(document = JSON.stringify(fixtureDocument), selected = false) {
  const state = createFixtureState();
  if (selected) {
    const snapshot = state.client.get('/snapshot/records') as ReadSnapshot;
    state.client.set('/ui/selection', { binding: 'records', identity: snapshot.object.identity.map(key => snapshot.rows[0][key]) });
  }
  return render(Surface, { props: { document, bindings: fixtureBindings, store: state.client, propose: async () => {} } }).body;
}
