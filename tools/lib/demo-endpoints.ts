// tools/lib/demo-endpoints.ts — read demo.toml, and turn a browser path into a real URL (#168).
//
// Shared by tools/demo-seed and tools/demo-record so the two cannot disagree about which
// capability a path belongs to.
//
// The resolution rule is not invented here. dashboard/vite.config.ts already proxies
// `/<capability>` to that capability's port with the prefix stripped, reading ports from
// `tools/capability.sh registry` — which reads them from each service.toml. This mirrors
// that rule against the same registry, so the demo reaches a capability at the same address
// the dashboard does, and adding a capability stays a service.toml edit. Writing a port in
// here would make this the second place a port lives, which is the one thing the manifest
// contract exists to prevent.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

export const SJEL_ROOT = resolve(dirname(new URL(import.meta.url).pathname), "../..");

export interface RegistryEntry {
  name: string;
  kind: string;
  scope: "capability" | "spine";
  port: string;
  health_path: string;
  ready_path: string;
  proxy_api_only: string;
  proxy_extra: string[];
}

/** A list→detail expansion: record one fixture per id found in an already-recorded list. */
export interface ExpandRule {
  from: string;
  id_field: string;
  into: string;
  /** The key holding the list when the body is an object, e.g. `entities` for `{ entities: [...] }`.
   *  Absent means the body is the list. */
  list_field?: string;
}

export interface DemoCapability {
  name: string;
  /** The generator in tools/demo-seed that fills it. Absent means "record as found". */
  writes?: string;
  paths: string[];
  expand: ExpandRule[];
}

export interface DemoManifest {
  seed: string;
  anchor: string;
  label: string;
  /** Where tools/demo-origin listens, and the only place that address is written down.
   *  Comms and Scouting are pointed at it by URL, Transit by its endpoint overrides. */
  origin: string;
  fixturesDir: string;
  capabilities: DemoCapability[];
  /** Capability name → why the demo does not include it. Rendered, not just recorded. */
  absent: Record<string, string>;
}

export function loadManifest(path = join(SJEL_ROOT, "demo/demo.toml")): DemoManifest {
  // Bun.TOML under the same documented exception tools/self.ts takes: tools/lib/toml.sh's
  // single-line grep contract cannot express arrays of tables, and this manifest is one.
  const raw = Bun.TOML.parse(readFileSync(path, "utf8")) as Record<string, any>;
  const demo = raw.demo ?? {};
  for (const field of ["seed", "anchor", "label", "origin"]) {
    if (typeof demo[field] !== "string" || demo[field] === "") {
      throw new Error(`demo.toml: [demo] ${field} is required`);
    }
  }
  if (!/^\d{4}-\d{2}-\d{2}$/.test(demo.anchor)) {
    throw new Error(`demo.toml: [demo] anchor must be YYYY-MM-DD, got ${demo.anchor}`);
  }
  // Loopback only, checked rather than trusted. The demo origin exists so three capabilities
  // can fetch something; an origin pointing off-machine would make the published corpus depend
  // on a host nobody in this repository controls, which is the failure the manifest's one
  // invariant is about.
  if (!/^http:\/\/127\.0\.0\.1:\d+$/.test(demo.origin)) {
    throw new Error(`demo.toml: [demo] origin must be http://127.0.0.1:<port>, got ${demo.origin}`);
  }

  const capabilities: DemoCapability[] = Object.entries(raw.capability ?? {}).map(
    ([name, value]) => {
      const entry = value as Record<string, any>;
      if (!Array.isArray(entry.paths) || entry.paths.length === 0) {
        throw new Error(`demo.toml: [capability.${name}] declares no paths`);
      }
      return {
        name,
        writes: typeof entry.writes === "string" ? entry.writes : undefined,
        paths: entry.paths as string[],
        expand: (entry.expand ?? []) as ExpandRule[],
      };
    },
  );
  if (capabilities.length === 0) throw new Error("demo.toml: no [capability.*] tables");

  const absent: Record<string, string> = {};
  for (const [name, value] of Object.entries(raw.absent ?? {})) {
    const reason = (value as Record<string, unknown>).reason;
    if (typeof reason !== "string" || reason.trim() === "") {
      throw new Error(`demo.toml: [absent.${name}] has no reason — say why, or delete it`);
    }
    absent[name] = reason.trim();
  }

  return {
    seed: demo.seed,
    anchor: demo.anchor,
    label: demo.label,
    origin: demo.origin,
    fixturesDir: raw.fixtures?.dir ?? "demo/fixtures",
    capabilities,
    absent,
  };
}

/**
 * The enabled capability set, as `tools/capability.sh registry` reports it.
 *
 * `overlayRoot` names which deployment to ask about, passed explicitly to the child rather
 * than left to whatever the calling shell exports. Callers that want "this machine" omit it;
 * the tests pass demo/overlay, because "does every path in demo.toml resolve" is a question
 * about the repository and must not depend on which capabilities the runner happens to enable.
 */
export function registry(overlayRoot?: string): RegistryEntry[] {
  const out = execFileSync(join(SJEL_ROOT, "tools/capability.sh"), ["registry"], {
    encoding: "utf8",
    env: overlayRoot ? { ...process.env, SJEL_OVERLAY_ROOT: overlayRoot } : process.env,
  });
  return JSON.parse(out) as RegistryEntry[];
}

/** The overlay the published demonstration runs as. */
export const DEMO_OVERLAY = join(SJEL_ROOT, "demo/overlay");

interface Route {
  /** The browser-side prefix, longest match wins. */
  prefix: string;
  capability: string;
  origin: string;
  /** Whether the proxy strips `/<name>` before forwarding. False for `proxy_extra` paths,
   *  which predate the uniform rule and pass through as written. */
  strip: boolean;
}

/** The proxy table, in the same shape and from the same source as the dev server's. */
export function routes(entries = registry()): Route[] {
  const table: Route[] = [];
  for (const svc of entries) {
    if (svc.scope === "spine" || !svc.port) continue;
    const origin = `http://127.0.0.1:${svc.port}`;
    table.push({
      prefix: svc.proxy_api_only === "true" ? `/${svc.name}/api` : `/${svc.name}`,
      capability: svc.name,
      origin,
      strip: true,
    });
    for (const extra of svc.proxy_extra ?? []) {
      table.push({ prefix: extra, capability: svc.name, origin, strip: false });
    }
  }
  // Longest prefix first: `/finance/api` must win over a hypothetical `/finance`, the same
  // way Vite's own middleware orders its table.
  return table.sort((a, b) => b.prefix.length - a.prefix.length);
}

/**
 * URL prefixes for capabilities the demo does NOT run.
 *
 * `tools/capability.sh registry` answers for the ENABLED set, which under the demo overlay is
 * the five capabilities being recorded. That is correct for resolving a URL — an unenabled
 * capability has no address — and wrong for the demo bundle, which has to recognise
 * `/comms/feed` as a capability call in order to answer it with "Comms is not in this demo,
 * here is why". Without these the shim would classify it as a static asset and let it reach
 * the network, where a Pages host answers 404 and the page reports a broken capability.
 *
 * Read straight from each service.toml, applying the same prefix rule as `routes()`. No port
 * is derived, because nothing will ever be fetched from these.
 */
export function absentPrefixes(names: string[]): Array<{ capability: string; prefix: string }> {
  const out: Array<{ capability: string; prefix: string }> = [];
  for (const name of names) {
    const path = join(SJEL_ROOT, "capabilities", name, "service.toml");
    if (!existsSync(path)) {
      throw new Error(`demo.toml: [absent.${name}] names no capability in this tree`);
    }
    const svc = Bun.TOML.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
    out.push({ capability: name, prefix: svc.proxy_api_only === "true" ? `/${name}/api` : `/${name}` });
    for (const extra of (svc.proxy_extra as string[] | undefined) ?? []) {
      out.push({ capability: name, prefix: extra });
    }
  }
  return out;
}

export interface Resolved {
  capability: string;
  url: string;
}

/** Where a browser path actually lives. Throws rather than guessing: an unroutable path in
 *  demo.toml is a typo, and answering it with a plausible URL would surface as an empty
 *  fixture nobody traces back here. */
export function resolvePath(browserPath: string, table: Route[]): Resolved {
  const match = table.find(
    (r) => browserPath === r.prefix || browserPath.startsWith(`${r.prefix}/`) || browserPath.startsWith(`${r.prefix}?`),
  );
  if (!match) {
    throw new Error(
      `demo: no capability serves '${browserPath}'. ` +
        `Known prefixes: ${table.map((r) => r.prefix).join(", ")}`,
    );
  }
  const tail = match.strip ? browserPath.slice(`/${match.capability}`.length) : browserPath;
  return { capability: match.capability, url: `${match.origin}${tail || "/"}` };
}

/** The file a browser path is recorded into, relative to the fixtures directory.
 *
 *  Readable rather than hashed, because the first thing anybody does with a fixture set is
 *  look for the one behind a wrong-looking screen. The browser never computes this — the
 *  recorder writes index.json and the shim looks paths up in it — so it is free to be a
 *  convenience for humans rather than a contract between two programs. */
export function fixtureFile(browserPath: string): string {
  const [path, query] = browserPath.split("?", 2);
  const segments = path.split("/").filter((s) => s !== "");
  // A `..` here is always a mistake in demo.toml, and it is not a harmless one: the recorder
  // mkdir -p's the fixture's parent and writes it, so a traversing path would put a file
  // outside the fixtures tree — and the tree is rm -rf'd on the next run. Refuse rather than
  // normalise, because a silently rewritten path records the right bytes under a name the
  // index then disagrees with.
  if (segments.some((s) => s === "." || s === "..")) {
    throw new Error(`demo: '${browserPath}' contains a relative segment; write the path out in full`);
  }
  const clean = segments.join("/") || "index";
  const suffix = query ? `__${query.replace(/[^A-Za-z0-9._=-]+/g, "-")}` : "";
  return `${clean}${suffix}.json`;
}
