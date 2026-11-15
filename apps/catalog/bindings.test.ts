import { expect, test } from 'bun:test';
import { resolveBinding, rowsFrom, linkedBinding, registry, readRows, linkedRows } from './src/bindings';
import type { Objects } from './src/objects';

const binding = { node: 'list', object: 'projects.project', read: 'list', parameters: {} };
const project: Objects['projects.project'] = { name: 'Fixture', archived: false, open: 1, done: 0, notes: 0, data_class: 'c1' };
test('approved reads encode values, retain class and preserve omitted content', () => {
  expect(rowsFrom(binding, { projects: [project] })).toEqual([project]);
  const object = registry.objects[0];
  const next = linkedBinding(object, object.links[0], { ...project, name: 'A & B/#' });
  expect(resolveBinding(next).url).toBe('/projects/api/project?content=false&name=A+%26+B%2F%23');
  const row = { path: 'Fixture/Hub.md', kind: 'project', title: 'Fixture', done: false, due: null, priority: 2, archived: false, data_class: 'c1', data_class_rationale: 'fixture' };
  expect(rowsFrom(next, { records: [row] })[0]).not.toHaveProperty('content');
});
test('invalid objects, writes and parameter injection name the failing node', () => {
  for (const invalid of [
    { ...binding, object: 'missing' },
    { ...binding, read: 'POST /api/record' },
    { ...binding, read: '/api/record/property' },
    { ...binding, parameters: { url: 'https://example.com' } },
  ]) expect(() => resolveBinding(invalid)).toThrow('list:');
});
test('missing classification and malformed aggregate responses fail closed', () => {
  expect(() => rowsFrom(binding, { projects: [{ name: 'Unclassified', archived: false }] })).toThrow('list:');
  expect(() => rowsFrom(binding, { projects: {} })).toThrow('list:');
});
test('failed bindings perform no fetch', async () => {
  const previous = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = (() => { calls++; throw new Error('unexpected request'); }) as typeof fetch;
  try {
    await expect(readRows({ ...binding, read: 'write' })).rejects.toThrow('list:');
    expect(calls).toBe(0);
  } finally { globalThis.fetch = previous; }
});
test('schema-invalid values and executable binding fields are refused', () => {
  expect(() => rowsFrom(binding, { projects: [{ ...project, archived: 'false' }] })).toThrow('list: invalid object properties');
  expect(() => rowsFrom(binding, { projects: [{ ...project, name: null }] })).toThrow('list: invalid object properties');
  expect(() => resolveBinding({ ...binding, script: 'alert(1)' } as typeof binding)).toThrow('list: invalid binding shape');
});

test('project identity filters keep active and archived records separate', () => {
  const link = registry.objects[0].links[0];
  const rows = [{ path: 'Live/Plan.md', archived: false }, { path: 'Archive/Live/Plan.md', archived: true }];
  expect(linkedRows(link, { archived: false }, rows)).toEqual([rows[0]]);
  expect(linkedRows(link, { archived: true }, rows)).toEqual([rows[1]]);
});
