import { resolve, sep } from 'node:path';
const root = resolve(import.meta.dirname, 'dist/client');
if (!await Bun.file(resolve(root, 'index.html')).exists()) throw new Error('Build the adapter evaluation first');
const server = Bun.serve({
  hostname: '127.0.0.1', port: 0,
  async fetch(request) {
    if (request.method !== 'GET') return new Response('Read only', { status: 405 });
    const url = new URL(request.url);
    let path: string;
    try { path = resolve(root, decodeURIComponent(url.pathname.slice(1)) || 'index.html'); }
    catch { return new Response('Invalid path', { status: 400 }); }
    if (!path.startsWith(root + sep)) return new Response('Not found', { status: 404 });
    const file = Bun.file(path);
    return await file.exists() ? new Response(file, { headers: { 'X-Content-Type-Options': 'nosniff' } }) : new Response('Not found', { status: 404 });
  },
});
console.log(`Adaptive catalog evaluation: http://127.0.0.1:${server.port}/`);
