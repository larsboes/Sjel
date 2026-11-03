#!/usr/bin/env bun
// website/scripts/serve.ts — serve the built static site using Bun.serve.
//
//   bun website/scripts/serve.ts [dir] [port]
import { join } from "node:path";

const dir = process.argv[2] ?? join(import.meta.dir, "../../site");
const port = Number(process.argv[3] ?? 8000);

const server = Bun.serve({
  port,
  async fetch(req) {
    let p = new URL(req.url).pathname;
    if (p.endsWith("/")) p += "index.html";
    const file = Bun.file(join(dir, p));
    if (await file.exists()) return new Response(file);
    const htmlFile = Bun.file(join(dir, p + ".html"));
    if (await htmlFile.exists()) return new Response(htmlFile);
    return new Response("Not found", { status: 404 });
  },
});

console.log(`Serving ${dir} at http://127.0.0.1:${server.port} (Ctrl+C to stop)`);
