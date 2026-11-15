/** Local browser fixture. Production assets and fictional data only; no capability access. */
import { resolve, sep } from 'node:path';
import fixture from './fixtures/projects.json';
const root = resolve(import.meta.dir, 'dist');
if (!await Bun.file(resolve(root, 'index.html')).exists()) throw new Error('Build the catalog first');
const server = Bun.serve({
  hostname: '127.0.0.1', port: 0,
  async fetch(request) {
    const url = new URL(request.url);
    if (request.method !== 'GET') return new Response('Read only', { status: 405 });
    if (url.pathname === '/projects/api/projects') return Response.json({ projects: fixture.projects });
    if (url.pathname === '/projects/api/project') return Response.json({ records: url.searchParams.get('content') === 'false' ? fixture.records.map(({ content, ...record }) => record) : fixture.records });
    if (url.pathname === '/projects/api/record') {
      const record = fixture.records.find(row => row.path === url.searchParams.get('path'));
      return record ? Response.json({ record }) : new Response('Missing record', { status: 404 });
    }
    if (!url.pathname.startsWith('/catalog/')) return new Response('Not found', { status: 404 });
    const path = resolve(root, decodeURIComponent(url.pathname.slice('/catalog/'.length)) || 'index.html');
    if (!path.startsWith(root + sep)) return new Response('Not found', { status: 404 });
    const file = Bun.file(path);
    return await file.exists() ? new Response(file) : new Response('Not found', { status: 404 });
  },
});
console.log(`Fictional catalog fixture: http://127.0.0.1:${server.port}/catalog/`);
