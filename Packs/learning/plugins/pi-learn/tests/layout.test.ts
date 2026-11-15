import { expect, test } from 'bun:test';
import { DEFAULT_LAYOUT, loadLayout, vaultLayout } from '../src/layout';

test('layout is vault-scoped and rejects traversal, overlap and misplaced logs', async () => {
  let value: unknown = { learning: 'Study', knowledge: 'Ideas', resources: 'Sources', log: 'Study/Log.md' };
  const app = { vault: { adapter: { exists: async () => true, read: async () => JSON.stringify(value) } } } as any;
  expect((await loadLayout(app)).knowledge).toBe('Ideas');
  for (const invalid of [
    { ...DEFAULT_LAYOUT, knowledge: '../Private' },
    { ...DEFAULT_LAYOUT, knowledge: '/Knowledge' },
    { ...DEFAULT_LAYOUT, knowledge: 'Learning/Knowledge' },
    { ...DEFAULT_LAYOUT, log: 'Resources/Log.md' },
  ]) {
    value = invalid;
    await expect(loadLayout(app)).rejects.toThrow();
    expect(vaultLayout(app).knowledge).toBe('Ideas');
  }
  expect(vaultLayout({} as any)).toEqual(DEFAULT_LAYOUT);
});
