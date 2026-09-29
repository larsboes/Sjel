// tools/self.test.ts — planted-fixture tests for the pure core of the self-model.
//
// Every case here is a failure this actually hit on 2026-07-30, not a hypothetical. The
// graph shapes are real: graphify emits an import specifier and its target file as two
// separate nodes, gives unqualified symbols one global node, and records foreign module
// names in the same field as internal paths. A rollup written against the naive
// assumption ("every node names a file, one node per file") over-counts every TS unit and
// invents coupling that does not exist.
// Run: bun test tools/self.test.ts

import { describe, expect, test } from "bun:test";
import {
  classifyPath,
  couplingFromCargo,
  couplingFromRustPath,
  mergeCoupling,
  rollUp,
  unitForPath,
} from "./lib/self-model.ts";

/** A virtual tree: only these paths "exist". */
const tree = (...paths: string[]) => {
  const set = new Set(paths);
  return (p: string) => set.has(p);
};

describe("classifyPath", () => {
  test("a path that exists verbatim is internal", () => {
    const c = classifyPath("capabilities/comms/src/main.rs", tree("capabilities/comms/src/main.rs"));
    expect(c.cls).toBe("internal");
    expect(c.path).toBe("capabilities/comms/src/main.rs");
  });

  test("existing Graphify memory is local, never internal", () => {
    const path = "graphify-out/memory/2026-08-02-query.md";
    expect(classifyPath(path, tree(), tree(path)).cls).toBe("local");
  });

  test("other ignored output below an internal root is local, not stale", () => {
    const path = "capabilities/comms/debug-cache.json";
    expect(classifyPath(path, tree(), tree(path)).cls).toBe("local");
  });

  test("tracked root documentation remains internal for the unmatched aggregate", () => {
    expect(classifyPath("README.md", tree("README.md"), tree("README.md"))).toMatchObject({
      cls: "internal",
      path: "README.md",
    });
  });

  test("an extension-stripped import specifier resolves to its real file", () => {
    // graphify records `dashboard/src/lib/api` for the node whose file is api.ts.
    const c = classifyPath("dashboard/src/lib/api", tree("dashboard/src/lib/api.ts"));
    expect(c.cls).toBe("internal");
    expect(c.path).toBe("dashboard/src/lib/api.ts");
  });

  test("a .svelte.ts module resolves, not just plain .ts", () => {
    const c = classifyPath(
      "dashboard/src/lib/capabilities.svelte",
      tree("dashboard/src/lib/capabilities.svelte.ts"),
    );
    expect(c.cls).toBe("internal");
    expect(c.path).toBe("dashboard/src/lib/capabilities.svelte.ts");
  });

  test("a bare package name is external, never stale", () => {
    expect(classifyPath("maplibre-gl", tree()).cls).toBe("external");
    expect(classifyPath("svelte", tree()).cls).toBe("external");
  });

  test("a $-alias is external even though it contains a slash", () => {
    expect(classifyPath("$app/navigation", tree()).cls).toBe("external");
  });

  test("an unresolvable path under a known root is stale — the one real defect", () => {
    const c = classifyPath("capabilities/deleted/src/gone.rs", tree());
    expect(c.cls).toBe("stale");
  });

  test("a null or empty source_file is its own class, not silently internal", () => {
    expect(classifyPath(null, tree()).cls).toBe("empty");
    expect(classifyPath("", tree()).cls).toBe("empty");
  });
});

describe("unitForPath", () => {
  test("maps each of the three nouns plus the spine directories", () => {
    expect(unitForPath("capabilities/comms/src/main.rs")).toEqual({ name: "comms", kind: "capability" });
    expect(unitForPath("libs/sjel-config/src/lib.rs")).toEqual({ name: "sjel-config", kind: "lib" });
    expect(unitForPath("Packs/writing/skills/x.md")).toEqual({ name: "writing", kind: "pack" });
    expect(unitForPath("dashboard/src/routes/+page.svelte")).toEqual({ name: "dashboard", kind: "spine" });
    expect(unitForPath("tools/doctor.ts")).toEqual({ name: "tools", kind: "spine" });
  });

  test("a root-level file belongs to no unit", () => {
    expect(unitForPath("README.md")).toBeNull();
    expect(unitForPath("axon.toml")).toBeNull();
  });
});

describe("rollUp", () => {
  test("a dangling relative import is foreign, while a deleted source stays stale", () => {
    const tracked = tree("Packs/harness/pi-packages/accordion/extension/mock-server.mjs");
    const r = rollUp(
      [
        {
          id: "dangling",
          label: "../core/ops.ts",
          source_file: "Packs/harness/pi-packages/accordion/core/ops.ts",
        },
        { id: "deleted", label: "ops.ts", source_file: "Packs/harness/deleted.ts" },
      ],
      tracked,
      tracked,
    );
    expect(r.buckets.stale).toEqual(["Packs/harness/deleted.ts"]);
    expect(r.buckets.external).toBe(1);
  });

  test("local Graphify artifacts do not change the public rollup", () => {
    const tracked = tree("capabilities/comms/src/main.rs");
    const local = "graphify-out/memory/private-query.md";
    const baseline = rollUp(
      [{ id: "code", source_file: "capabilities/comms/src/main.rs" }],
      tracked,
      tracked,
    );
    const withLocal = rollUp(
      [
        { id: "code", source_file: "capabilities/comms/src/main.rs" },
        { id: "local", source_file: local },
      ],
      tracked,
      tree("capabilities/comms/src/main.rs", local),
    );

    expect(withLocal.units).toEqual(baseline.units);
    expect(withLocal.buckets).toEqual(baseline.buckets);
    expect(withLocal.admittedNodes).toBe(baseline.admittedNodes);
  });

  test("a specifier and its target file count as ONE file but TWO nodes", () => {
    // The 2026-07-30 finding: 16 files existed as two nodes each. Prefix matching still
    // put both in the right unit, so unit assignment was safe — but every per-unit file
    // count was inflated. Both numbers are reported so the gap stays visible.
    const r = rollUp(
      [
        { id: "a", source_file: "dashboard/src/lib/api" },
        { id: "b", source_file: "dashboard/src/lib/api.ts" },
      ],
      tree("dashboard/src/lib/api.ts"),
    );
    expect(r.units).toHaveLength(1);
    expect(r.units[0]).toMatchObject({ name: "dashboard", kind: "spine", files: 1, nodes: 2 });
  });

  test("external, empty, stale and unmatched land in named buckets, never in a unit", () => {
    const r = rollUp(
      [
        { id: "1", source_file: "svelte" },
        { id: "2", source_file: "$app/state" },
        { id: "3", source_file: null },
        { id: "4", source_file: "capabilities/gone/src/x.rs" },
        { id: "5", source_file: "README.md" },
        { id: "6", source_file: "capabilities/comms/src/main.rs" },
      ],
      tree("capabilities/comms/src/main.rs", "README.md"),
    );
    expect(r.buckets.external).toBe(2);
    expect(r.buckets.empty).toBe(1);
    expect(r.buckets.stale).toEqual(["capabilities/gone/src/x.rs"]);
    expect(r.buckets.unmatched).toEqual(["README.md"]);
    expect(r.units.map((u) => u.name)).toEqual(["comms"]);
  });

  test("units come back sorted, so two runs on one graph are byte-identical", () => {
    const nodes = [
      { id: "1", source_file: "capabilities/transit/a.rs" },
      { id: "2", source_file: "capabilities/comms/a.rs" },
      { id: "3", source_file: "libs/sjel-config/a.rs" },
    ];
    const exists = tree(...nodes.map((n) => n.source_file!));
    expect(rollUp(nodes, exists).units.map((u) => u.name)).toEqual([
      "comms",
      "sjel-config",
      "transit",
    ]);
  });
});

describe("couplingFromRustPath", () => {
  test("a #[path] include reaching another unit is coupling", () => {
    const edges = couplingFromRustPath(
      "capabilities/calendar/src/lib.rs",
      '#[path = "../../../libs/sjel-config/src/lib.rs"]\npub(crate) mod sjel_config;',
    );
    expect(edges).toHaveLength(1);
    expect(edges[0]).toMatchObject({ from: "calendar", to: "sjel-config", kind: "rust-path" });
  });

  test("a #[path] include staying inside its own unit is not coupling", () => {
    const edges = couplingFromRustPath(
      "capabilities/calendar/src/lib.rs",
      '#[path = "./helpers/thing.rs"] mod thing;',
    );
    expect(edges).toEqual([]);
  });

  test("plain `use` statements are ignored — they name crates, not units", () => {
    // This is why graphify's import edges were unusable: `use std::sync::OnceLock` says
    // nothing about which unit owns anything.
    const edges = couplingFromRustPath(
      "capabilities/sjel-status/src/main.rs",
      "use std::sync::OnceLock;\nuse axum::Router;\nuse serde_json::json;",
    );
    expect(edges).toEqual([]);
  });

  test("a doc comment naming another unit is not coupling", () => {
    // The near-miss that killed text-substring confirmation: sjel-status/src/main.rs
    // mentions "scouting" only in a `//!` comment about retired port literals, and a
    // whole-file substring check accepted it as a real import.
    const edges = couplingFromRustPath(
      "capabilities/sjel-status/src/main.rs",
      "//! also retired the hardcoded transit/scouting port literals this file used to carry",
    );
    expect(edges).toEqual([]);
  });
});

describe("couplingFromCargo", () => {
  test("a path dependency naming another unit is coupling", () => {
    const edges = couplingFromCargo(
      "capabilities/sjel-status/Cargo.toml",
      [
        "[dependencies]",
        'sjel-server = { path = "../../libs/sjel-server" }',
        'sjel-config = { path = "../../libs/sjel-config" }',
      ].join("\n"),
    );
    expect(edges.map((e) => e.to).sort()).toEqual(["sjel-config", "sjel-server"]);
  });

  test("registry dependencies are not unit coupling", () => {
    const edges = couplingFromCargo(
      "capabilities/comms/Cargo.toml",
      '[dependencies]\naxum = "0.7"\nserde = { version = "1.0", features = ["derive"] }',
    );
    expect(edges).toEqual([]);
  });

  test("a [lib] or [[bin]] path inside the same unit is not coupling", () => {
    const edges = couplingFromCargo(
      "capabilities/transit/Cargo.toml",
      '[lib]\npath = "src/lib.rs"\n\n[[bin]]\nname = "transit-server"\npath = "src/server.rs"',
    );
    expect(edges).toEqual([]);
  });

  test("a dev-dependency reaches just as far as a dependency", () => {
    const edges = couplingFromCargo(
      "capabilities/trips/Cargo.toml",
      '[dev-dependencies]\nstation-time = { path = "../../libs/station-time" }',
    );
    expect(edges.map((e) => e.to)).toEqual(["station-time"]);
  });
});

describe("mergeCoupling", () => {
  test("both evidence kinds are kept per pair, so a one-sided pair stays visible", () => {
    // A pair backed by cargo-dep alone is the expected shape for a crate that is linked
    // but whose modules no source file includes by #[path]. Keeping the kinds is what
    // lets a reader tell that apart from drift instead of guessing.
    const merged = mergeCoupling([
      { from: "calendar", to: "sjel-config", kind: "rust-path", file: "a.rs", evidence: "x" },
      { from: "calendar", to: "sjel-config", kind: "cargo-dep", file: "Cargo.toml", evidence: "y" },
      { from: "scouting", to: "transit", kind: "cargo-dep", file: "Cargo.toml", evidence: "z" },
    ]);
    expect(merged).toHaveLength(2);
    expect(merged[0]).toMatchObject({ from: "calendar", to: "sjel-config", kinds: ["cargo-dep", "rust-path"] });
    expect(merged[1]).toMatchObject({ from: "scouting", to: "transit", kinds: ["cargo-dep"] });
  });

  test("output is sorted, so the committed artifact is stable", () => {
    const merged = mergeCoupling([
      { from: "trips", to: "sjel-server", kind: "rust-path", file: "a", evidence: "x" },
      { from: "comms", to: "sjel-server", kind: "rust-path", file: "b", evidence: "y" },
      { from: "comms", to: "sjel-config", kind: "rust-path", file: "c", evidence: "z" },
    ]);
    expect(merged.map((m) => `${m.from}->${m.to}`)).toEqual([
      "comms->sjel-config",
      "comms->sjel-server",
      "trips->sjel-server",
    ]);
  });
});

// generateWouldDropCode and generateWouldBakeStaleGraph were tested here until 2026-09-29. Both
// guarded `tools/self generate` against writing an artifact whose per-unit `code` counts had been
// dropped or rolled up from a stale graph, and both are gone with the committed code layer — it is
// fused on read now, so `generate` writes the same tracked-file layers on every machine and there
// is no refusal left to probe. tools/self.test.sh watches the behaviour that replaced them: a
// graphless generate succeeding, and the artifact carrying no code layer to drop.
