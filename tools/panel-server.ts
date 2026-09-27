#!/usr/bin/env bun
// panel-server.ts — serves one capability's built static site, and knows whether
// anybody is looking at it.
//
// Static panels use SvelteKit with adapter-static: each build is a directory of files,
// and the Vite dev server they use during development would otherwise be
// buying hot-reload for a reader who is not editing. Two dev servers cost 229 MB and
// hold permanent fs watchers on node_modules; this costs a Bun process and no watchers.
//
// The second job is the one that could not be done from outside. An idle reaper needs
// to know "is somebody reading this", and after dropping the dev server there is no
// HMR websocket left to count — a static server sees no traffic at all from a page
// that is simply open. So this injects a heartbeat into the HTML it serves: the panel
// projects contain zero lines about any of this, and every future panel gets it for
// free by being served here. That is the whole reason the injection exists; without it
// the reaper would kill a page mid-read.
//
// usage: bun tools/panel-server.ts <dist-dir>       # dir relative to SJEL_ROOT, or absolute
// env:   SJEL_PORT   the port to bind (exported by tools/service-runner.sh from the manifest)

import { existsSync, statSync } from "node:fs";
import { isAbsolute, join, normalize, resolve, sep } from "node:path";

const HEARTBEAT_INTERVAL_MS = 30_000;

const axonRoot = process.env.SJEL_ROOT ?? process.cwd();
const arg = process.argv[2];
if (!arg) {
  console.error("panel-server.ts: needs the directory to serve — usage: bun tools/panel-server.ts <dist-dir>");
  process.exit(1);
}
const root = resolve(isAbsolute(arg) ? arg : join(axonRoot, arg));
if (!existsSync(join(root, "index.html"))) {
  console.error(`panel-server.ts: ${root} has no index.html — has the panel been built?`);
  process.exit(1);
}

const port = Number(process.env.SJEL_PORT ?? 0);
if (!Number.isInteger(port) || port <= 0) {
  console.error(`panel-server.ts: SJEL_PORT must be a port number, got ${JSON.stringify(process.env.SJEL_PORT)}`);
  process.exit(1);
}

// Seeded at boot, not at epoch: a panel the dashboard just started has not been read
// yet, and reporting it as maximally idle would let the reaper stop it before the tab
// it was started for has finished loading.
let lastSignal = Date.now();

const CONTENT_TYPES: Record<string, string> = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".ico": "image/x-icon",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".txt": "text/plain; charset=utf-8",
  ".webmanifest": "application/manifest+json",
  ".webp": "image/webp",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};

const contentType = (path: string): string => {
  const dot = path.lastIndexOf(".");
  return (dot === -1 ? undefined : CONTENT_TYPES[path.slice(dot)]) ?? "application/octet-stream";
};

/**
 * The visibility heartbeat, injected before </head>.
 *
 * `visibilityState`, not "the tab exists": a panel left open in a background tab for
 * three days is not being read, and the point of the whole exercise is that it stops
 * costing anything. A visible tab pings on load, on every visibility change, and on an
 * interval well under any sane idle timeout. sendBeacon survives the page going away.
 */
const HEARTBEAT = `<script>
(function () {
  var ping = function () {
    if (document.visibilityState !== "visible") return;
    if (navigator.sendBeacon) navigator.sendBeacon("/__axon/ping");
    else fetch("/__axon/ping", { method: "POST", keepalive: true }).catch(function () {});
  };
  ping();
  setInterval(ping, ${HEARTBEAT_INTERVAL_MS});
  document.addEventListener("visibilitychange", ping);
})();
</script>`;

/**
 * A way back to the shell, injected for the same reason the heartbeat is: a panel is
 * opened in its own tab on its own port, and without this it is a leaf you can only
 * leave with the back button. Every panel gets it, including ones that do not exist
 * yet, and no panel project contains a line about it.
 *
 * The port comes from the environment (service-runner reads dashboard/service.toml, the
 * one place it lives); the HOST comes from the browser's `location`, because the
 * dashboard reached over Tailscale is not on 127.0.0.1 — the same trap api.ts's
 * panelUrl documents from the other direction. No env var means no link rather than a
 * guessed one, so a panel started by hand is simply linkless.
 */
const shellPort = process.env.SJEL_SHELL_PORT ?? "";
const BACKLINK = /^\d+$/.test(shellPort)
  ? `<a id="axon-back" href="#" title="Back to Axon">← Axon</a>
<style>
#axon-back {
  position: fixed; left: 1rem; bottom: 1rem; z-index: 2147483647;
  padding: .4rem .7rem; border-radius: 999px;
  font: 500 12px/1 ui-sans-serif, system-ui, sans-serif; text-decoration: none;
  color: #e6f6ff; background: rgba(15, 23, 42, .82); border: 1px solid rgba(56, 189, 248, .45);
  backdrop-filter: blur(6px); opacity: .55; transition: opacity .15s ease;
}
#axon-back:hover, #axon-back:focus-visible { opacity: 1; }
@media print { #axon-back { display: none; } }
</style>
<script>
document.getElementById("axon-back").href =
  location.protocol + "//" + location.hostname + ":${shellPort}/";
</script>`
  : "";

/**
 * Resolve a URL path to a file inside `root`, or null.
 *
 * Normalised and then re-checked against the root prefix, so "..%2f..%2fetc/passwd"
 * cannot walk out of the served directory — this listens on loopback, but a path
 * traversal in a static server is not a bug worth having either way.
 */
function resolveFile(urlPath: string): string | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(urlPath);
  } catch {
    return null;
  }
  const candidate = normalize(join(root, decoded));
  if (candidate !== root && !candidate.startsWith(root + sep)) return null;

  for (const path of [candidate, `${candidate}.html`, join(candidate, "index.html")]) {
    if (existsSync(path) && statSync(path).isFile()) return path;
  }
  return null;
}

/**
 * A precompressed sibling, when the client asked for that encoding.
 *
 * adapter-static is configured with `precompress: true`, so .br and .gz already sit
 * next to every asset. HTML is deliberately excluded: it gets the heartbeat injected,
 * and serving the precompressed copy would ship the one file that must be rewritten.
 */
function precompressed(path: string, accept: string): { path: string; encoding: string } | null {
  if (path.endsWith(".html")) return null;
  for (const [encoding, ext] of [["br", ".br"], ["gzip", ".gz"]] as const) {
    if (accept.includes(encoding) && existsSync(path + ext)) return { path: path + ext, encoding };
  }
  return null;
}

Bun.serve({
  hostname: "127.0.0.1", // loopback only, same contract as libs/axon-server (README.md#security-and-data)
  port,
  async fetch(request) {
    const url = new URL(request.url);

    // The liveness pair. `ping` is the browser saying a visible tab exists; `idle`
    // is the reaper asking. Reading the state must never look like using it, so
    // /__axon/idle deliberately does not touch lastSignal — otherwise a poll every
    // 30s would keep every panel alive forever.
    if (url.pathname === "/__axon/ping") {
      lastSignal = Date.now();
      return new Response(null, { status: 204 });
    }
    if (url.pathname === "/__axon/idle") {
      return Response.json({
        idle_seconds: Math.floor((Date.now() - lastSignal) / 1000),
        last_signal: new Date(lastSignal).toISOString(),
      });
    }

    const file = resolveFile(url.pathname) ?? join(root, "index.html");

    if (file.endsWith(".html")) {
      const html = await Bun.file(file).text();
      // Before </head> when there is one — the script must run before the app's own
      // JS can throw, or a broken panel would also be an unreapable one.
      const withHeartbeat = html.includes("</head>")
        ? html.replace("</head>", `${HEARTBEAT}</head>`)
        : html + HEARTBEAT;
      // The backlink goes last, after the app's own markup, so nothing it renders can
      // sit on top of the one control that gets you out of the panel.
      const injected = withHeartbeat.includes("</body>")
        ? withHeartbeat.replace("</body>", `${BACKLINK}</body>`)
        : withHeartbeat + BACKLINK;
      return new Response(injected, {
        headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-cache" },
      });
    }

    const encoded = precompressed(file, request.headers.get("accept-encoding") ?? "");
    return new Response(Bun.file(encoded?.path ?? file), {
      headers: {
        "content-type": contentType(file),
        ...(encoded ? { "content-encoding": encoded.encoding } : {}),
        // _app/immutable is content-hashed by Vite; everything else is the page shell.
        "cache-control": file.includes(`${sep}immutable${sep}`)
          ? "public, max-age=31536000, immutable"
          : "no-cache",
      },
    });
  },
});

console.log(`panel-server: serving ${root} on 127.0.0.1:${port}`);
