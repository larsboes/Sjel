#!/usr/bin/env bun
// tools/demo-check-paths.ts — does every path demo.toml declares exist, and did it get
// recorded (#168).
//
// WHAT THIS EXISTS TO CATCH. `demo/demo.toml` names browser paths; `tools/demo-record` GETs
// them and writes `index.json`; the published bundle's fetch shim serves a request only if
// that path is a key in the index, and answers 501 otherwise (dashboard/src/lib/demo.ts). So
// a path in the manifest that no capability serves has exactly one symptom today, and it is
// a 501 on the published site — read by a visitor, not by a build.
//
// `tools/lib/demo-endpoints.test.ts` already asserts that every declared path RESOLVES. That
// is a question about prefixes: `/sjel-status/api/sjel-status/upstreams` resolves, because
// `/sjel-status` is a prefix sjel-status owns. It has not been a route since 2026-08-28, when
// PRD Q41 retired `tools/upstream-checker` and the endpoint went with it — and the manifest
// carried it for eleven days, because resolving and being served are different questions.
//
// TWO HALVES, and the first one is the one that runs everywhere:
//
//   declared    Every path matches a route the owning capability's own `const ROUTES` table
//               declares — the manifest `GET /routes` serves, which ISA PLC-1 already keeps
//               equal to the router (each capability's `every_route_the_router_serves_is_
//               declared` test). Pure file reading: no capability runs, no port is bound, no
//               network. This is the half that would have caught /upstreams the day it left.
//
//   recorded    With --fixtures <dir>: every declared path is a key in that recording's
//               index, and the file it names is on disk. Answers the question the first half
//               cannot — a recording that is older than the manifest it is published beside.
//
// Not folded into demo-record's own loop, where it would be tautological: that loop BUILDS
// the index from these paths, so it agrees with the manifest by construction. The mismatch
// is between a manifest and a recording made at a different time.

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

import {
  SJEL_ROOT,
  DEMO_OVERLAY,
  loadManifest,
  registry,
  resolvePath,
  routes,
  type DemoManifest,
} from "./lib/demo-endpoints.ts";

/** A `{id}` in an `expand` rule stands for a generated id, which is one path segment. */
const ID_PLACEHOLDER = "an-id";

/**
 * The paths one capability declares it serves, read from its `const ROUTES` table.
 *
 * Rust source rather than a running `GET /routes`, because the whole point is to answer
 * without the stack up — and the two cannot drift: each capability's own
 * `every_route_the_router_serves_is_declared` test fails when a `.route()` has no entry here.
 *
 * The parse is deliberately blunt: take the table's text and keep every string literal that
 * starts with `/`. A method is "GET" or "POST" and a summary is a sentence, so neither can be
 * mistaken for a path, and it survives every shape rustfmt wraps the table into — `r("GET",
 * "/health", ...)` on one line, the same call over four, and the long `route_manifest::Route {
 * method: ..., path: ... }` struct form trips uses.
 */
export function declaredRoutes(capability: string, root = SJEL_ROOT): string[] {
  const src = join(root, "capabilities", capability, "src");
  if (!existsSync(src)) {
    throw new Error(`demo.toml names '${capability}', which has no capabilities/${capability}/src`);
  }
  const holders = rustFiles(src).filter((file) => readFileSync(file, "utf8").includes("const ROUTES"));
  if (holders.length === 0) {
    throw new Error(`${capability} declares no 'const ROUTES' table; nothing here can say what it serves`);
  }
  // Refused rather than merged. Two tables in one capability means "which one is the surface"
  // is a guess, and a gate that guesses is worse than no gate.
  if (holders.length > 1) {
    throw new Error(`${capability} has more than one 'const ROUTES' table: ${holders.join(", ")}`);
  }
  const text = readFileSync(holders[0], "utf8");
  const start = text.indexOf("const ROUTES");
  const end = text.indexOf("\n];", start);
  if (end < 0) throw new Error(`${capability}: its 'const ROUTES' table has no terminating '];'`);
  return [...text.slice(start, end).matchAll(/"(\/[^"]*)"/g)].map((match) => match[1]);
}

function rustFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir).sort()) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...rustFiles(path));
    else if (path.endsWith(".rs")) out.push(path);
  }
  return out;
}

/** Whether a concrete request path is served by a declared route, `:param` segments and all. */
export function servedBy(requestPath: string, declared: string[]): boolean {
  const asked = requestPath.split("/").filter((s) => s !== "");
  return declared.some((route) => {
    const parts = route.split("/").filter((s) => s !== "");
    if (parts.length !== asked.length) return false;
    return parts.every((part, index) => part.startsWith(":") || part === asked[index]);
  });
}

/** Every browser path the manifest asks for: the literal ones, plus each expansion target. */
export function declaredPaths(manifest: DemoManifest): Array<{ capability: string; path: string }> {
  const out: Array<{ capability: string; path: string }> = [];
  for (const cap of manifest.capabilities) {
    for (const path of cap.paths) out.push({ capability: cap.name, path });
    // An `into` template is a path this recording will request once per row in the list it
    // expands, so a template naming a route nobody serves fails the same way — later, and
    // per id, which is a worse place to find it.
    for (const rule of cap.expand) {
      out.push({ capability: cap.name, path: rule.into.replaceAll("{id}", ID_PLACEHOLDER) });
    }
  }
  return out;
}

/**
 * The offline half. Returns one sentence per path no capability declares a route for.
 *
 * Asked of `demo/overlay`, not of this machine, for the reason
 * `tools/lib/demo-endpoints.test.ts` gives: whether the manifest is coherent is a fact about
 * the repository, and a workstation with a different capability set must not change the answer.
 */
export function undeclaredPaths(manifest: DemoManifest, root = SJEL_ROOT): string[] {
  const table = routes(registry(DEMO_OVERLAY));
  const cache = new Map<string, string[]>();
  const problems: string[] = [];
  for (const { capability, path } of declaredPaths(manifest)) {
    const resolved = resolvePath(path, table);
    const served = new URL(resolved.url).pathname;
    if (!cache.has(resolved.capability)) {
      cache.set(resolved.capability, declaredRoutes(resolved.capability, root));
    }
    const declared = cache.get(resolved.capability)!;
    if (!servedBy(served, declared)) {
      problems.push(
        `[capability.${capability}] declares '${path}', which reaches ` +
          `${resolved.capability} as '${served}' — a path its own route manifest does not ` +
          `declare. Nothing can record it, so the published demo answers 501.`,
      );
    }
  }
  return problems;
}

interface FixtureIndex {
  routes: Record<string, string>;
}

/**
 * The recorded half. Returns one sentence per declared path the recording does not hold.
 *
 * Both conditions, because they fail differently: a path missing from `routes` is a shim
 * lookup that misses (501), and a path present with a file that is not on disk is a fetch for
 * a fixture the bundle does not carry (404 from the Pages host, which reads as a broken
 * capability). `expand` targets are skipped: their ids come from a body recorded at run time,
 * so the manifest cannot name them and neither can this.
 */
export function unrecordedPaths(manifest: DemoManifest, fixturesDir: string): string[] {
  const indexPath = join(fixturesDir, "index.json");
  if (!existsSync(indexPath)) {
    return [`no recording at ${indexPath} — run tools/demo-record`];
  }
  const index = JSON.parse(readFileSync(indexPath, "utf8")) as FixtureIndex;
  const recorded = index.routes ?? {};
  const problems: string[] = [];
  for (const cap of manifest.capabilities) {
    for (const path of cap.paths) {
      const file = recorded[path];
      if (!file) {
        problems.push(
          `[capability.${cap.name}] declares '${path}', which this recording's index.json ` +
            `has no entry for — the published demo answers 501 for it.`,
        );
        continue;
      }
      if (!existsSync(join(fixturesDir, file))) {
        problems.push(
          `[capability.${cap.name}] declares '${path}', whose fixture '${file}' is named by ` +
            `index.json and is not in ${fixturesDir}.`,
        );
      }
    }
  }
  return problems;
}

function main(): void {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) {
    console.log("tools/demo-check-paths [--fixtures <dir>]");
    return;
  }
  const at = args.indexOf("--fixtures");
  const fixtures = at >= 0 ? args[at + 1] : undefined;
  if (at >= 0 && !fixtures) {
    console.error("demo-check-paths: --fixtures needs a directory");
    process.exit(1);
  }

  const manifest = loadManifest();
  const problems = [
    ...undeclaredPaths(manifest),
    ...(fixtures ? unrecordedPaths(manifest, fixtures) : []),
  ];
  if (problems.length > 0) {
    for (const problem of problems) console.error(`demo-check-paths: ${problem}`);
    console.error(`demo.toml path check FAILED (${problems.length}).`);
    process.exit(1);
  }
  const counted = declaredPaths(manifest).length;
  console.log(
    `demo.toml path check passed (${counted} declared path(s) across ` +
      `${manifest.capabilities.length} capabilities, each served by a declared route` +
      (fixtures ? `; every listed path recorded in ${fixtures}` : "") +
      ").",
  );
}

if (import.meta.main) {
  try {
    main();
  } catch (err) {
    console.error(`demo-check-paths: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  }
}
