import fixture from '../fixtures/projects.json';
import type { Binding } from '../src/bindings';
import { createViewState } from './runtime';
import type { ViewDocument } from './document';

export const fixtureBindings: Record<string, Binding> = {
  records: { node: 'records', object: 'projects.record', read: 'project', parameters: { name: 'Kitchen renovation' } },
};
export const fixtureDocument: ViewDocument = {
  version: 1, root: 'root', elements: {
    root: { type: 'Stack', props: {}, children: ['list', 'detail', 'draft'] },
    list: { type: 'ObjectList', props: { binding: 'records' }, children: [] },
    detail: { type: 'ObjectDetail', props: { binding: 'records' }, children: [] },
    draft: { type: 'Draft', props: { binding: 'records' }, children: [] },
  },
};

/** Supply fictional records through the same classified binding boundary as a host. */
export function createFixtureState() {
  return createViewState(fixtureBindings, { records: { records: fixture.records.filter(record => !record.archived) } });
}
