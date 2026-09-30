import { execFileSync, execSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig, type Plugin, type ProxyOptions } from "vite";
import {
  hasSameOrigin,
  installCommsProxyAuthorization,
  isMutation,
  loadCommsProxyCredential,
} from "./vite/comms-proxy-auth.ts";

const SJEL_ROOT = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const port = Number(process.env.SJEL_PORT ?? 47117);
// What every visitor downloads before interacting with anything.
const APP_BUNDLE_LIMIT_BYTES = 500_000;
// A chunk nobody can reach without a dynamic import is not application weight, but it is
// still weight. Capped separately and higher: a diagram renderer is legitimately large, and
// what this guards against is one arriving unnoticed rather than one existing at all.
const LAZY_CHUNK_LIMIT_BYTES = 1_200_000;
// Renderers heavy enough that reaching the eager graph would be a regression nobody notices
// until every page is slow. One row per library rather than a second copy of the rule.
//
// `total` bounds the library across every chunk Rollup splits it into, measured with ~5%
// headroom so an upstream bump that doubles something has to be looked at. It is a
// footprint bound, not a per-load one, and the two differ by a lot for Mermaid: MapLibre
// arrives as one real chunk of 1.01 MB plus a 0.49 MB worker. Both halves grew with the
// 2026-09-30 bump to v6.11.2, which measured 1,573,065 bytes against the 1,530,000 v6.4.1
// had set (0.98 MB + 0.46 MB there; v5 was ~1.05 MB for the chunk alone) -- +2.8% across
// four minor releases, and entirely inside the lazy graph, so the eager limit above is
// untouched by it. The bound moved rather than the dependency: Q77 rolls on the latest, and
// what this number exists to catch is a doubling nobody saw, not a 2.8% nobody would.
// Mermaid self-splits by diagram type across 52
// chunks totalling 2.57 MB, of which a reader pulls the ~1.3 MB core plus only the diagram
// types actually on the page. What bounds any single download is LAZY_CHUNK_LIMIT_BYTES
// above; this bounds the library growing while nobody is watching.
//
// `assets` is why a footprint bound has to name more than modules. Vite builds a worker in a
// SEPARATE Rollup pass and emits the result as an asset, so its modules never appear in any
// `chunk.modules` of this bundle and `match` cannot see them. MapLibre's worker is 0.46 MB --
// a third of the library -- and it went completely unmeasured the day it started being built.
// A budget with a third of its subject invisible is worse than no budget, because it reads
// green.
const LAZY_VENDORS = [
  {
    label: "MapLibre",
    match: ["/maplibre-gl/"],
    assets: [/maplibre-gl-worker.*\.js$/],
    total: 1_650_000,
  },
  { label: "Mermaid", match: ["/mermaid/", "/@mermaid-js/"], assets: [], total: 2_700_000 },
];

interface RegistryEntry {
  name: string;
  kind: string;
  scope: "capability" | "spine";
  port: string;
  proxy_extra: string[];
  proxy_api_only: string;
}

// Lazy is a property of the import graph, not of a flag.
//
// This guard used to ask `chunk.isDynamicEntry`, and measure every chunk that was not
// MapLibre against the application limit. Both are wrong in the same direction, and adding
// Mermaid is what exposed it: `isDynamicEntry` is true only for a chunk that IS the target
// of a dynamic import, so a shared chunk Rollup splits OUT of one comes back false while
// still being unreachable without it. Mermaid produced exactly that -- three chunks totalling
// 2.2 MB, none of them reachable from an entry, all of them failing a limit named for the
// application. The lazy loading was correct; the measurement was not.
//
// So reachability is computed rather than asked for: walk `imports` from each entry chunk
// and never follow `dynamicImports`, because not following one is the entire point. What the
// walk reaches is what every visitor downloads, and that is what the application limit is
// about. Everything else is bounded too, just at a size a renderer can actually be.
function bundleGuard(): Plugin {
  return {
    name: "bundle-guard",
    generateBundle(_options, bundle) {
      const eager = new Set<string>();
      // How each eager chunk was reached, so a failure can print the chain instead of only the
      // verdict. "reachable from an entry" without the path is a sentence that costs whoever
      // reads it an afternoon.
      const reachedVia = new Map<string, string>();
      const walk = (fileName: string, from?: string) => {
        if (eager.has(fileName)) return;
        const chunk = bundle[fileName];
        if (!chunk || chunk.type !== "chunk") return;
        eager.add(fileName);
        if (from) reachedVia.set(fileName, from);
        for (const dep of chunk.imports) walk(dep, fileName);
      };
      const chainTo = (fileName: string): string => {
        const chain = [fileName];
        for (let at = reachedVia.get(fileName); at; at = reachedVia.get(at)) chain.unshift(at);
        return chain.join("\n    -> ");
      };
      for (const output of Object.values(bundle)) {
        if (output.type === "chunk" && output.isEntry) walk(output.fileName);
      }

      const allChunks = Object.values(bundle).flatMap((output) =>
        output.type === "chunk" ? [output] : [],
      );
      const sizeOf = (chunk: (typeof allChunks)[number]) => Buffer.byteLength(chunk.code);

      for (const chunk of allChunks) {
        const isEager = eager.has(chunk.fileName);
        const limit = isEager ? APP_BUNDLE_LIMIT_BYTES : LAZY_CHUNK_LIMIT_BYTES;
        const bytes = sizeOf(chunk);
        if (bytes > limit) {
          this.error(
            `${chunk.fileName} is ${bytes} bytes; the limit for a ` +
              `${isEager ? "statically reachable" : "lazy"} chunk is ${limit}.`,
          );
        }
      }

      // MapLibre without its worker is not a slow map, it is a dead one: no tile is parsed, no
      // GeoJSON is processed, the canvas stays blank and the frame reads "Loading map…" forever
      // with no error and no failed request. It reached the served bundle exactly that way,
      // because MapLibre asks for its worker through a TEMPLATE literal
      // (`new URL(\`./${name}\`, import.meta.url)`) that Rollup cannot follow, so nothing was
      // emitted and the missing path fell through sjel-status' SPA fallback as 200 text/html.
      //
      // src/lib/map/surface.ts hands MapLibre a Vite-built worker instead. This asserts the
      // build actually produced one, because the runtime symptom of its absence is silence.
      const hasMapLibre = allChunks.some((chunk) =>
        Object.keys(chunk.modules).some((id) => id.includes("/maplibre-gl/")),
      );
      const hasWorker = Object.keys(bundle).some((fileName) =>
        /maplibre-gl-worker.*\.js$/.test(fileName),
      );
      if (hasMapLibre && !hasWorker) {
        this.error(
          "MapLibre is in the bundle but its worker asset is not. Without it every map renders " +
            "a blank canvas and never fires `load`, silently. Check the " +
            "`maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url` import in src/lib/map/surface.ts.",
        );
      }

      for (const vendor of LAZY_VENDORS) {
        const owned = allChunks.filter((chunk) =>
          Object.keys(chunk.modules).some((id) =>
            vendor.match.some((fragment) => id.includes(fragment)),
          ),
        );
        if (owned.length === 0) continue;

        const leaked = owned.find((chunk) => eager.has(chunk.fileName));
        if (leaked) {
          const owner = Object.keys(leaked.modules).find((id) =>
            vendor.match.some((fragment) => id.includes(fragment)),
          );
          this.error(
            `${vendor.label} must remain lazy; ${leaked.fileName} is reachable from an entry ` +
              `without a dynamic import. It carries ${owner ?? "the library"}, and the static ` +
              `chain that reaches it is:\n    ${chainTo(leaked.fileName)}`,
          );
        }

        // Chunks the module matcher found, plus separately-built assets it structurally cannot.
        const assetBytes = Object.values(bundle)
          .filter(
            (output) =>
              output.type === "asset" && vendor.assets.some((re) => re.test(output.fileName)),
          )
          .reduce(
            (total, output) =>
              total + Buffer.byteLength((output as { source: string | Uint8Array }).source),
            0,
          );
        const workerChunkBytes = allChunks
          .filter((chunk) => vendor.assets.some((re) => re.test(chunk.fileName)))
          .reduce((total, chunk) => total + sizeOf(chunk), 0);
        const bytes =
          owned.reduce((total, chunk) => total + sizeOf(chunk), 0) + assetBytes + workerChunkBytes;
        if (bytes > vendor.total) {
          this.error(
            `${vendor.label} bundles total ${bytes} bytes; the limit is ${vendor.total}.`,
          );
        }
      }
    },
  };
}

// The proxy table is derived, not written. `tools/capability.sh registry` reads the
// service.toml manifests through tools/lib/toml.sh, the shell-side TOML parser
// (CONTRIBUTING.md#one-manifest-per-concern), so a capability's port is declared in exactly one file and this config,
// sjel-status and the runner all read the same number.
//
// Read once at dev-server start: enabling a capability or moving a port means
// restarting the dashboard, which is honest — the shell's shape follows the machine's.
function registry(): RegistryEntry[] {
  const out = execFileSync(resolve(SJEL_ROOT, "tools/capability.sh"), ["registry"], {
    encoding: "utf8",
  });
  return JSON.parse(out) as RegistryEntry[];
}

function buildProxy(): Record<string, ProxyOptions> {
  const proxy: Record<string, ProxyOptions> = {};
  const commsCredential = loadCommsProxyCredential(SJEL_ROOT);

  if (!commsCredential.authorization) {
    console.warn(
      `[dashboard] Comms proxy has no credential (${commsCredential.reason}); every route except /health and /ready remains fail-closed.`,
    );
  }

  for (const svc of registry()) {
    // The spine is this process; a capability with no port has no HTTP surface.
    if (svc.scope === "spine" || !svc.port) continue;
    const target = `http://127.0.0.1:${svc.port}`;

    // Uniform rule, no manifest field needed: /<name> reaches the capability with the
    // prefix stripped, so a capability's own contract never has to know it is proxied.
    const proxyPath =
      svc.proxy_api_only === "true" ? `/${svc.name}/api` : `/${svc.name}`;
    const options: ProxyOptions = {
      target,
      changeOrigin: true,
      rewrite: (path) => path.replace(new RegExp(`^/${svc.name}`), ""),
    };
    if (svc.name === "comms" && commsCredential.authorization) {
      const authorization = commsCredential.authorization;
      options.configure = (server) => installCommsProxyAuthorization(server, authorization);
    }
    proxy[proxyPath] = options;

    // Surfaces whose paths predate that rule (transit's /api, scouting's /discover)
    // pass through unstripped, declared per capability in its own manifest.
    for (const extra of svc.proxy_extra ?? []) {
      proxy[extra] = { target, changeOrigin: true };
    }
  }

  // macmon used to be hand-proxied here, on a hardcoded 9911, because it was not a
  // capability. It is capabilities/macmon now, so the registry loop above already gives it
  // /macmon from its own `port` — one fewer place for that number to be wrong.

  return proxy;
}

/** Top memory consumers on this machine, as JSON. Runs `ps` every request, so this
 *  is intentionally not a capability — it is a trivial dev-server convenience for the
 *  dashboard's Systems page hover detail. The response is ~500 bytes, uncached.
 *  
 *  Each entry: { pid, rss_mb, name }. `name` is the basename of the executable. */
function topProcesses(_req: import("http").IncomingMessage, res: import("http").ServerResponse) {
  try {
    const raw = execSync("ps -eo pid=,rss=,comm=", { encoding: "utf8", timeout: 3000 });
    const lines = raw.trim().split("\n");
    const procs = lines
      .map((l) => {
        const m = l.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
        if (!m) return null;
        const rssKb = parseInt(m[2], 10);
        // Filter out the ps process itself and tiny system processes
        if (rssKb < 100 * 1024) return null; // < 100 MB
        return { pid: parseInt(m[1], 10), rss_mb: Math.round(rssKb / 1024), name: m[3].split("/").pop() ?? m[3] };
      })
      .filter((p): p is NonNullable<typeof p> => p !== null)
      .sort((a, b) => b.rss_mb - a.rss_mb);
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(procs.slice(0, 15)));
  } catch {
    res.writeHead(500);
    res.end(JSON.stringify({ error: "failed to read processes" }));
  }
}

function guardCommsMutations(
  req: import("http").IncomingMessage,
  res: import("http").ServerResponse,
  next: () => void,
) {
  if (!req.url?.startsWith("/comms/") || !isMutation(req.method) || hasSameOrigin(req.headers)) {
    next();
    return;
  }
  res.writeHead(403, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ error: "cross-origin Comms mutations are not allowed" }));
}

// A function, not an object, so the proxy table is built only when a server is
// actually starting. `buildProxy()` shells out to tools/capability.sh and reads
// every manifest in the repo; a production build has no server and no need for the
// table. Evaluating it at config load made `bun run build` depend on the whole manifest
// tree to produce static files.
// This shell is reached over the tailnet as well as over loopback. `tailscale serve`
// terminates TLS on the machine's own MagicDNS name and proxies to 127.0.0.1, so the Host
// header arrives as `<machine>.<tailnet>.ts.net` and Vite's DNS-rebinding guard refuses it
// with "Blocked request. This host is not allowed." On a phone, that message is the whole
// page — the dashboard looks broken rather than unreachable.
//
// A suffix rather than this machine's name, deliberately. The specific MagicDNS name is a
// house fact and this repo is public; `.ts.net` is a Tailscale-wide fact that identifies
// nobody. The leading dot is Vite's own "this domain and any subdomain" form. Anything that
// is not a tailnet name comes from the overlay through SJEL_DASHBOARD_ALLOWED_HOSTS,
// comma-separated, so a deployment fact stays in the deployment.
//
// What deliberately does not change: the listen address stays 127.0.0.1. Tailscale is what
// makes this reachable and it authenticates at the tailnet layer. Binding wider to "fix"
// the same symptom would hand this surface to whatever network the laptop joins next.
const allowedHosts = [
  ".ts.net",
  ...(process.env.SJEL_DASHBOARD_ALLOWED_HOSTS ?? "")
    .split(",")
    .map((host) => host.trim())
    .filter(Boolean),
];

export default defineConfig(({ command }) => ({
  // Substituted as a literal so `if (!DEMO)` in src/lib/demo.ts folds at build time and the
  // fetch shim leaves no trace in a normal bundle. Declared here rather than left to Vite's
  // .env loading, because the value comes from the environment tools/demo-site exports and
  // `import.meta.env` reads .env FILES — a shell variable would silently be undefined, which
  // fails in the safe direction (no demo) and is therefore the kind of bug that ships.
  define: {
    "import.meta.env.VITE_SJEL_DEMO": JSON.stringify(process.env.SJEL_DEMO === "1" ? "1" : "0"),
  },
  plugins: [sveltekit(), bundleGuard(), {
    name: "top-processes",
    configureServer(server) {
      server.middlewares.use(guardCommsMutations);
      server.middlewares.use("/api/top-processes", topProcesses);
    },
  }],
  server: {
    host: "127.0.0.1",
    port,
    strictPort: true,
    allowedHosts,
    ...(command === "serve" ? { proxy: buildProxy() } : {}),
  },
  preview: { host: "127.0.0.1", port, strictPort: true, allowedHosts },
  // maplibre-gl spawns its worker with `new URL("maplibre-gl-worker.mjs",
  // import.meta.url)`. Pre-bundled into .vite/deps that URL 404s (the optimizer
  // emits no worker file), the worker dies with an opaque error, and every map
  // hangs at "Loading map…" — dev only; the production build resolves the
  // worker correctly. Excluding it makes dev serve the real ESM from
  // node_modules, where import.meta.url points at the shipped worker.
  optimizeDeps: { exclude: ["maplibre-gl"] },
  // MapLibre spawns its worker with `new Worker(url, { type: "module" })`, so the worker Vite
  // emits for it has to be an ES module too. The default is IIFE, which a module worker
  // refuses to run. Nothing else in this shell has a worker, so this is not a project-wide
  // compromise -- it is the only worker's actual format.
  worker: { format: "es" },
  build: {
    // Vite's own warning, silenced up to the largest size bundleGuard() will actually
    // allow — otherwise it fires on every lazy renderer chunk the guard has already
    // measured and accepted, and a warning that is always on is one nobody reads.
    // The guard, not this number, is what fails a build.
    chunkSizeWarningLimit: LAZY_CHUNK_LIMIT_BYTES / 1000,
  },
}));
