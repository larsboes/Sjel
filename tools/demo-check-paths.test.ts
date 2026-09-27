// Tests for tools/demo-check-paths.ts.
//
// Every case here plants an input the gate should refuse and watches it refuse. A check that
// answers the same for a known-bad manifest as for the committed one is measuring something
// else, and this repository has an entry about that (PRD §13.1, sixth silent failure).
//
// The planted manifests are written to a temp file and loaded through the real `loadManifest`,
// so the fixture goes through the same parser the recorder does.

import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  declaredRoutes,
  servedBy,
  undeclaredPaths,
  unrecordedPaths,
} from "./demo-check-paths.ts";
import { SJEL_ROOT, loadManifest } from "./lib/demo-endpoints.ts";

/** A manifest with one capability block, otherwise the committed file's own header values. */
function manifestWith(paths: string[], capability = "sjel-status", extra = ""): string {
  return [
    '[demo]',
    'seed = "axon-demo-v1"',
    'anchor = "2026-03-16"',
    'label = "Demo data — generated, not real"',
    'origin = "http://127.0.0.1:8099"',
    '',
    '[fixtures]',
    'dir = "demo/fixtures"',
    '',
    `[capability.${capability}]`,
    `paths = [${paths.map((p) => `"${p}"`).join(", ")}]`,
    extra,
  ].join("\n");
}

function planted(body: string) {
  const dir = mkdtempSync(join(tmpdir(), "axon-demo-check-"));
  const path = join(dir, "demo.toml");
  writeFileSync(path, `${body}\n`);
  return { dir, manifest: loadManifest(path) };
}

describe("the committed demo.toml", () => {
  test("declares only paths a capability's own route manifest serves", () => {
    // The regression this file exists for. `/api/sjel-status/upstreams` sat in this list for
    // eleven days after PRD Q41 deleted the handler, resolving correctly the whole time.
    expect(undeclaredPaths(loadManifest())).toEqual([]);
  });
});

describe("undeclaredPaths", () => {
  test("refuses a path no capability serves, and names where it would have gone", () => {
    const { dir, manifest } = planted(
      manifestWith(["/sjel-status/api/sjel-status/health", "/sjel-status/api/sjel-status/upstreams"]),
    );
    try {
      const problems = undeclaredPaths(manifest);
      expect(problems).toHaveLength(1);
      expect(problems[0]).toContain("/sjel-status/api/sjel-status/upstreams");
      expect(problems[0]).toContain("as '/api/sjel-status/upstreams'");
      expect(problems[0]).toContain("501");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("refuses an expand rule whose target is not a route, not only a literal path", () => {
    const { dir, manifest } = planted(
      manifestWith(
        ["/trips/api/plans"],
        "trips",
        'expand = [{ from = "/trips/api/plans", id_field = "id", into = "/trips/api/plans/{id}/invented" }]',
      ),
    );
    try {
      const problems = undeclaredPaths(manifest);
      expect(problems).toHaveLength(1);
      expect(problems[0]).toContain("invented");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("accepts a real expand target, so the refusal above is about the path and not the rule", () => {
    const { dir, manifest } = planted(
      manifestWith(
        ["/trips/api/plans"],
        "trips",
        'expand = [{ from = "/trips/api/plans", id_field = "id", into = "/trips/api/plans/{id}/cost" }]',
      ),
    );
    try {
      expect(undeclaredPaths(manifest)).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("unrecordedPaths", () => {
  const fixtures = () => {
    const dir = mkdtempSync(join(tmpdir(), "axon-demo-fixtures-"));
    mkdirSync(join(dir, "sjel-status", "api", "sjel-status"), { recursive: true });
    writeFileSync(join(dir, "sjel-status/api/sjel-status/health.json"), "{}\n");
    return dir;
  };

  test("passes when the recording holds every declared path", () => {
    const dir = fixtures();
    const { dir: manifestDir, manifest } = planted(
      manifestWith(["/sjel-status/api/sjel-status/health"]),
    );
    try {
      writeFileSync(
        join(dir, "index.json"),
        JSON.stringify({
          routes: {
            "/sjel-status/api/sjel-status/health": "sjel-status/api/sjel-status/health.json",
          },
        }),
      );
      expect(unrecordedPaths(manifest, dir)).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
      rmSync(manifestDir, { recursive: true, force: true });
    }
  });

  test("refuses a declared path the index has no entry for — the 501 on the site", () => {
    const dir = fixtures();
    const { dir: manifestDir, manifest } = planted(
      manifestWith([
        "/sjel-status/api/sjel-status/health",
        "/sjel-status/api/sjel-status/capabilities",
      ]),
    );
    try {
      writeFileSync(
        join(dir, "index.json"),
        JSON.stringify({
          routes: {
            "/sjel-status/api/sjel-status/health": "sjel-status/api/sjel-status/health.json",
          },
        }),
      );
      const problems = unrecordedPaths(manifest, dir);
      expect(problems).toHaveLength(1);
      expect(problems[0]).toContain("/sjel-status/api/sjel-status/capabilities");
      expect(problems[0]).toContain("501");
    } finally {
      rmSync(dir, { recursive: true, force: true });
      rmSync(manifestDir, { recursive: true, force: true });
    }
  });

  test("refuses an index entry naming a fixture that is not on disk", () => {
    const dir = fixtures();
    const { dir: manifestDir, manifest } = planted(
      manifestWith(["/sjel-status/api/sjel-status/health"]),
    );
    try {
      writeFileSync(
        join(dir, "index.json"),
        JSON.stringify({
          routes: { "/sjel-status/api/sjel-status/health": "sjel-status/api/sjel-status/gone.json" },
        }),
      );
      const problems = unrecordedPaths(manifest, dir);
      expect(problems).toHaveLength(1);
      expect(problems[0]).toContain("gone.json");
    } finally {
      rmSync(dir, { recursive: true, force: true });
      rmSync(manifestDir, { recursive: true, force: true });
    }
  });

  test("a missing recording is a refusal, never a pass", () => {
    // The failure mode a "skip if absent" gate would have: green on the machine that never
    // recorded anything, which is every machine but the one that publishes.
    const { dir, manifest } = planted(manifestWith(["/sjel-status/api/sjel-status/health"]));
    try {
      expect(unrecordedPaths(manifest, join(dir, "nothing-here"))).toHaveLength(1);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("declaredRoutes", () => {
  test("reads the table through every shape rustfmt wraps it into", () => {
    // One line per entry (sjel-status), four lines per entry (finance), and the long struct
    // form (trips) — the three shapes in this tree, asserted so a reformat cannot quietly
    // empty the parse and turn every check above green.
    expect(declaredRoutes("sjel-status")).toContain("/api/sjel-status/capabilities/:name/start");
    expect(declaredRoutes("sjel-status")).not.toContain("/api/sjel-status/upstreams");
    expect(declaredRoutes("finance")).toContain("/__axon/freshness");
    expect(declaredRoutes("trips")).toContain("/api/plans/:id/cost");
    for (const capability of ["sjel-status", "calendar", "comms", "finance", "scouting", "transit", "trips"]) {
      expect(declaredRoutes(capability).length).toBeGreaterThan(3);
    }
  });

  test("refuses a capability with no route table rather than reporting it clean", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-demo-noroutes-"));
    try {
      mkdirSync(join(root, "capabilities", "invented", "src"), { recursive: true });
      writeFileSync(join(root, "capabilities/invented/src/main.rs"), "fn main() {}\n");
      expect(() => declaredRoutes("invented", root)).toThrow(/declares no 'const ROUTES' table/);
      expect(() => declaredRoutes("absent", root)).toThrow(/has no capabilities/);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("is read from the tree, not from a copy — SJEL_ROOT is the default", () => {
    expect(SJEL_ROOT.endsWith("/tools")).toBe(false);
    expect(declaredRoutes("finance", SJEL_ROOT).length).toBeGreaterThan(20);
  });
});

describe("servedBy", () => {
  test("matches a :param segment and refuses a longer or shorter path", () => {
    const declared = ["/api/plans", "/api/plans/:id/cost", "/health"];
    expect(servedBy("/api/plans", declared)).toBe(true);
    expect(servedBy("/api/plans/an-id/cost", declared)).toBe(true);
    expect(servedBy("/api/plans/an-id", declared)).toBe(false);
    expect(servedBy("/api/plans/an-id/cost/extra", declared)).toBe(false);
    expect(servedBy("/api/plan", declared)).toBe(false);
  });
});
