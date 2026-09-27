// dashboard/src/lib/demo.ts — the published demonstration's data source (#168).
//
// In a demo build this replaces `fetch` for capability calls, answering them from the fixture
// set tools/demo-record captured off real, synthetically seeded servers. Everything else about
// the app is the app: the same components, the same api.ts, the same error rendering. A demo
// built out of a second, simpler UI would demonstrate the second UI.
//
// Tree-shaken out of a normal build. `import.meta.env.VITE_SJEL_DEMO` is a literal at compile
// time, so `if (!DEMO) return` becomes `if (true) return` and Rollup drops the rest — this file
// costs a production bundle nothing, which is why it may sit in the eager import graph.
//
// WHAT A FAILED REQUEST SAYS, and why there are three answers instead of one. A demo is a
// partial system by construction, and the difference between "this is not in the demo",
// "this should be here and is missing" and "you cannot write from a static page" is the
// difference between a page that explains itself and a page that looks broken. So:
//
//   absent capability   503 + the reason from demo.toml, which api.ts renders verbatim as
//                       "comms: Feed items enter Comms only through a collector…"
//   demoed, no fixture  501 + the path, plus a console error. A gap in demo.toml, not a
//                       property of the demo — it should be loud in both places.
//   any mutation        403 read-only. The button stays, the click is honest.
//   cross-origin        403 blocked. A published page makes no third-party requests; that is
//                       a house rule, and leaving the shim to enforce it means a component
//                       that grows one cannot quietly ship it.

export const DEMO = import.meta.env.VITE_SJEL_DEMO === "1";

export interface DemoIndex {
  seed: string;
  anchor: string;
  label: string;
  absent: Record<string, string>;
  prefixes: Array<{ capability: string; prefix: string }>;
  routes: Record<string, string>;
}

let index: DemoIndex | null = null;

/** The manifest, once the shim is installed. Null in a normal build. */
export const demoIndex = (): DemoIndex | null => index;

const json = (status: number, body: unknown): Response =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

/** Strip origin and SvelteKit base, leaving the path api.ts asked for. */
function browserPath(input: string, base: string): string | null {
  let path: string;
  try {
    const url = new URL(input, location.href);
    if (url.origin !== location.origin) return null; // cross-origin, handled by the caller
    path = `${url.pathname}${url.search}`;
  } catch {
    return null;
  }
  return base && path.startsWith(base) ? path.slice(base.length) || "/" : path;
}

/** Which capability a path belongs to, by longest declared prefix. */
function owner(path: string, prefixes: DemoIndex["prefixes"]): string | null {
  const sorted = [...prefixes].sort((a, b) => b.prefix.length - a.prefix.length);
  const hit = sorted.find(
    (p) => path === p.prefix || path.startsWith(`${p.prefix}/`) || path.startsWith(`${p.prefix}?`),
  );
  return hit ? hit.capability : null;
}

/**
 * Install the shim. Called once from the root layout's load, which SvelteKit runs before any
 * page load — so no component can reach the network ahead of it.
 *
 * `base` is SvelteKit's configured base path: on Pages the demo is served from a subdirectory,
 * so `/finance/api/dashboard` arrives as `/Sjel/demo/finance/api/dashboard` and has to be
 * stripped back before it can be looked up. Passed in rather than imported, because $app/paths
 * is the layout's dependency and this file stays a plain module.
 */
/**
 * Pins the page's clock to the demo's anchor date, running forward from there.
 *
 * The recording is made around the anchor (demo.toml `anchor`): trips start days after it and the
 * calendar fixtures cover a window around it. The pages ask for ranges around "now", so with the
 * real clock a demo ages: on 2026-09-27 its March recording showed no upcoming trip and five
 * sources as unavailable. `new Date()` and `Date.now()` answer anchor time instead; a date built
 * from explicit arguments is unchanged.
 */
function installDemoClock(anchor: string): void {
  const RealDate = Date;
  const pinned = RealDate.parse(`${anchor}T09:00:00`);
  if (Number.isNaN(pinned)) return;
  const loadedAt = RealDate.now();
  const now = () => pinned + (RealDate.now() - loadedAt);
  class DemoDate extends RealDate {
    constructor(...args: unknown[]) {
      if (args.length === 0) super(now());
      else super(...(args as [number]));
    }
    static override now(): number {
      return now();
    }
  }
  globalThis.Date = DemoDate as DateConstructor;
}

export async function installDemoFetch(base: string): Promise<DemoIndex> {
  const real = globalThis.fetch.bind(globalThis);
  const res = await real(`${base}/fixtures/index.json`);
  if (!res.ok) {
    throw new Error(
      `demo: no fixture index at ${base}/fixtures/index.json (${res.status}). ` +
        "The bundle was built with VITE_SJEL_DEMO=1 but tools/demo-record never ran.",
    );
  }
  index = (await res.json()) as DemoIndex;
  const manifest = index;
  installDemoClock(manifest.anchor);

  globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw =
      typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();

    const path = browserPath(raw, base);
    if (path === null) {
      return json(403, {
        error: "This demo makes no requests to other hosts. Nothing was sent.",
      });
    }

    // The app's own assets and the fixtures themselves. Anything the demo index does not
    // claim a prefix for is not a capability call and belongs to the static bundle.
    const capability = owner(path, manifest.prefixes);
    if (capability === null) return real(input as RequestInfo, init);

    if (method !== "GET") {
      return json(403, {
        error: `${manifest.label}. This page is a recording, so nothing can be changed from it.`,
      });
    }

    const reason = manifest.absent[capability];
    if (reason) return json(503, { error: reason });

    const file = manifest.routes[path];
    if (!file) {
      // Loud on purpose. A demo route nobody recorded is a hole in demo.toml, and the only
      // moment anyone will notice is while looking at the page it broke.
      console.error(`demo: no fixture for ${path} — add it to [capability.${capability}] paths in demo.toml`);
      return json(501, {
        error: `not part of this demo recording (${path})`,
      });
    }
    return real(`${base}/fixtures/${file}`);
  };

  return manifest;
}
