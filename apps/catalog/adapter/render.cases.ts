import { expect, test } from 'bun:test';
const { renderFixture } = await import('./dist/server/render-entry.js');

test('real renderer uses the existing object components and shows a proposal draft', () => {
  const html = renderFixture();
  expect(html).toContain('Measure the room');
  expect(html).toContain('Select an object');
  expect(html).toContain('Proposal note');
  expect(html).toContain('Prepare proposal');
});

test('selected record content remains escaped text', () => {
  const html = renderFixture(undefined, true);
  expect(html).toContain('&lt;script');
  expect(html).not.toContain('<script>Never execute');
});

test('invalid documents produce a visible node error before renderer dispatch', () => {
  const html = renderFixture(JSON.stringify({ version: 1, root: 'bad', elements: { bad: { type: 'script', props: {}, children: [] } } }));
  expect(html).toContain('role="alert"');
  expect(html).toContain('bad:');
  expect(html).not.toContain('Proposal note');
});
