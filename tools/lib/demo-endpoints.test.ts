// Tests for tools/lib/demo-endpoints.ts (#168).
//
// The property that matters here is that a browser path resolves to the SAME capability the
// dashboard's dev-server proxy would send it to. Those are two implementations of one rule
// (dashboard/vite.config.ts and this module), both reading tools/capability.sh registry, and
// if they drift the demo records one capability's answer under another's path — which looks
// like working software right up until somebody reads the page.

import { describe, expect, test } from "bun:test";
import { DEMO_OVERLAY, fixtureFile, loadManifest, localDate, registry, resolvePath, routes, type RegistryEntry } from "./demo-endpoints.ts";

/** A registry standing in for three real shapes: the uniform rule, the api-only rule, and a
 *  capability with a legacy unstripped prefix. */
const REGISTRY: RegistryEntry[] = [
  { name: "finance", kind: "process", scope: "capability", port: "8090", health_path: "/health", ready_path: "/ready", proxy_api_only: "true", proxy_extra: [] },
  { name: "vault", kind: "process", scope: "capability", port: "8094", health_path: "/health", ready_path: "/ready", proxy_api_only: "", proxy_extra: [] },
  { name: "transit", kind: "process", scope: "capability", port: "3000", health_path: "/health", ready_path: "", proxy_api_only: "", proxy_extra: ["/api"] },
  { name: "tools", kind: "process", scope: "spine", port: "", health_path: "", ready_path: "", proxy_api_only: "", proxy_extra: [] },
  { name: "store", kind: "data", scope: "capability", port: "", health_path: "", ready_path: "", proxy_api_only: "", proxy_extra: [] },
];

const TABLE = routes(REGISTRY);

describe("routes", () => {
  test("skips the spine and anything with no HTTP surface", () => {
    const named = TABLE.map((r) => r.capability);
    expect(named).not.toContain("tools");
    expect(named).not.toContain("store");
  });

  test("orders longest prefix first, so /finance/api cannot lose to /finance", () => {
    const lengths = TABLE.map((r) => r.prefix.length);
    expect([...lengths].sort((a, b) => b - a)).toEqual(lengths);
  });
});

describe("resolvePath", () => {
  test("strips the capability prefix, matching the dev proxy's rewrite", () => {
    expect(resolvePath("/finance/api/dashboard", TABLE)).toEqual({
      capability: "finance",
      url: "http://127.0.0.1:8090/api/dashboard",
    });
    expect(resolvePath("/vault/api/tasks", TABLE).url).toBe("http://127.0.0.1:8094/api/tasks");
  });

  test("keeps a query string intact", () => {
    expect(resolvePath("/vault/api/tasks?status=open", TABLE).url).toBe(
      "http://127.0.0.1:8094/api/tasks?status=open",
    );
  });

  test("passes a proxy_extra prefix through unstripped", () => {
    // transit's /api predates the uniform rule and is served at that path, not under /transit.
    expect(resolvePath("/api/suggest?q=gent", TABLE)).toEqual({
      capability: "transit",
      url: "http://127.0.0.1:3000/api/suggest?q=gent",
    });
  });

  test("throws on a path no capability serves, rather than guessing", () => {
    // A typo in demo.toml has to fail here. Resolving it to a plausible URL would surface as
    // an empty fixture that nobody traces back to the manifest.
    expect(() => resolvePath("/nope/api/thing", TABLE)).toThrow(/no capability serves/);
  });

  test("does not treat a longer capability name as a prefix match", () => {
    expect(() => resolvePath("/vaultwarden/api", TABLE)).toThrow();
  });
});

describe("fixtureFile", () => {
  test("mirrors the path, so a wrong-looking screen is findable on disk", () => {
    expect(fixtureFile("/finance/api/dashboard")).toBe("finance/api/dashboard.json");
  });

  test("keeps a query in the filename, sanitized", () => {
    expect(fixtureFile("/calendar/api/entries?from=2026-02-01&to=2026-05-01")).toBe(
      "calendar/api/entries__from=2026-02-01-to=2026-05-01.json",
    );
  });

  test("gives two different queries two different files", () => {
    expect(fixtureFile("/vault/api/tasks?status=open")).not.toBe(
      fixtureFile("/vault/api/tasks?status=done"),
    );
  });

  // The recorder mkdir -p's a fixture's parent and writes it, and rm -rf's the tree on the
  // next run. A traversing path would therefore write, and later delete, outside it.
  test("refuses a path that would escape the fixtures directory", () => {
    expect(() => fixtureFile("/a/../../etc/passwd")).toThrow(/relative segment/);
    expect(() => fixtureFile("/finance/./api")).toThrow(/relative segment/);
  });

  test("normalises empty segments and never yields an absolute path", () => {
    for (const path of ["//a//b", "/a/b/", "/x?q=../../y"]) {
      const file = fixtureFile(path);
      expect(file.startsWith("/")).toBe(false);
      expect(file).not.toContain("//");
    }
  });
});

describe("the committed demo.toml", () => {
  const manifest = loadManifest();

  // Asked of the overlay the demo actually runs on, not of this machine. `capability.sh
  // registry` answers for the ENABLED set, so without pinning it the two cases below were
  // green on a workstation with these five capabilities on and red in CI, where the seed
  // overlay enables none. Whether every declared path resolves is a fact about the
  // repository; it must not depend on the runner.
  const live = routes(registry(DEMO_OVERLAY));

  test("declares a fixed seed and resolves the anchor to a date", () => {
    expect(manifest.seed).not.toBe("");
    expect(manifest.anchor).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  test("SJEL_DEMO_ANCHOR pins the anchor, and a bad value is refused", () => {
    const before = process.env.SJEL_DEMO_ANCHOR;
    try {
      process.env.SJEL_DEMO_ANCHOR = "2026-03-16";
      expect(loadManifest().anchor).toBe("2026-03-16");
      process.env.SJEL_DEMO_ANCHOR = "next week";
      expect(() => loadManifest()).toThrow(/YYYY-MM-DD/);
    } finally {
      if (before === undefined) delete process.env.SJEL_DEMO_ANCHOR;
      else process.env.SJEL_DEMO_ANCHOR = before;
    }
  });

  test("localDate is the local calendar day", () => {
    expect(localDate(new Date(2026, 9, 28, 23, 59))).toBe("2026-10-28");
  });

  test("every declared path resolves against the real registry", () => {
    // The one assertion that catches a capability being renamed or losing its port: it runs
    // against tools/capability.sh registry, not against the fixture above.
    for (const cap of manifest.capabilities) {
      for (const path of cap.paths) expect(() => resolvePath(path, live)).not.toThrow();
    }
  });

  test("a path is declared under the capability that serves it", () => {
    for (const cap of manifest.capabilities) {
      for (const path of cap.paths) {
        expect(resolvePath(path, live).capability).toBe(cap.name);
      }
    }
  });

  test("no two paths collide on one fixture file", () => {
    const all = manifest.capabilities.flatMap((c) => c.paths).map(fixtureFile);
    expect(new Set(all).size).toBe(all.length);
  });

  test("every expand rule names one of its own capability's paths", () => {
    for (const cap of manifest.capabilities) {
      for (const rule of cap.expand) {
        expect(cap.paths).toContain(rule.from);
        expect(rule.into).toContain("{id}");
      }
    }
  });

  test("an absent capability carries a reason, and is not also recorded", () => {
    const recorded = new Set(manifest.capabilities.map((c) => c.name));
    for (const [name, reason] of Object.entries(manifest.absent)) {
      expect(reason.length).toBeGreaterThan(40);
      expect(recorded.has(name)).toBe(false);
    }
  });
});
