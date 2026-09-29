// tools/lib/self-model.ts — the pure core of Axon's self-model: rolling graphify's
// file-and-function graph up to unit-level facts, and reading real compile-time coupling
// out of the two places that actually declare it.
//
// No I/O lives here on purpose. Every function takes its world as an argument (the node
// list, a path-resolver, a file's text) so tools/self.test.ts can probe the logic against
// fixtures instead of against this machine's checkout. tools/self.ts is the only file
// that reads disk, shells out, or writes self.json.
//
// One thing this file deliberately does NOT do: derive coupling from graphify's own
// import edges. That was built and then removed — on this corpus every cross-unit import
// edge it produced was a phantom, because graphify gives each unqualified symbol ONE
// global node owned by whichever file it extracted first, so every
// `use std::collections::BTreeMap` became an edge into whichever capability happened to
// own `btreemap`. It also missed all 20 files of real capability→lib coupling. Coupling
// therefore comes from `#[path]` attributes and Cargo path dependencies, which are literal
// strings in tracked files. Evidence: MEMORY/WORK/axon-self-model/ISA.md changelog,
// 2026-07-30. The second kind read Bazel labels until 2026-08-25 (PRD Q44); the manifests
// changed, the property did not.
//
// Why a separate node identity rather than reusing graphify's: graphify slugifies node
// ids from the ABSOLUTE scan path, so they can read
// `users_example_developer_axon_...`. Committing any of those into a public-safe repo
// would leak the home path, which is why graphify-out/ is git-ignored (see
// tools/graphify.sh). The rollup therefore emits unit names and counts only — never a
// graphify id.

/** The three nouns of CONTRIBUTING.md#three-architectural-nouns, plus the two spine directories that hold code. */
export type UnitKind = "capability" | "lib" | "spine" | "pack";

export interface Unit {
  name: string;
  kind: UnitKind;
}

/**
 * Extensions tried when a `source_file` names a module rather than a file.
 *
 * graphify records the import specifier as written, so `dashboard/src/lib/api` is the
 * node for `api.ts` and `dashboard/src/lib/capabilities.svelte` is the node for
 * `capabilities.svelte.ts`. `.svelte.ts` must be tried before `.ts` would ever match,
 * and it is listed explicitly because appending `.ts` to `...capabilities.svelte`
 * happens to be the correct answer only by coincidence of that naming convention.
 */
const RESOLVE_EXTENSIONS = [".ts", ".svelte.ts", ".tsx", ".js", ".mjs", ".svelte", ".rs", ".py"];
const RESOLVE_INDEXES = ["/index.ts", "/index.js", "/mod.rs"];

/** Top-level directories that make a path Axon-internal. Anything else is foreign. */
const INTERNAL_ROOTS = ["capabilities", "libs", "dashboard", "Packs", "tools", "schemas"];

export type PathClass = "internal" | "external" | "stale" | "empty" | "local";

export interface ClassifiedPath {
  cls: PathClass;
  /** The resolved on-disk path — set only when cls === "internal". */
  path?: string;
  /** What graphify actually wrote, kept for reporting. */
  raw: string;
}

/**
 * Classify one `source_file` into exactly one of four states, resolving extensions.
 *
 * The four-way split exists because "this path is not a file" conflates three
 * different things and only one of them is a defect:
 *   - internal: a tracked file in this checkout (possibly after extension resolution)
 *   - external: a foreign module specifier — a bare package name or a `$`-alias.
 *     Legitimate graph content; a dependency graph is supposed to have foreign nodes.
 *   - stale: looks internal (lives under a known root) but resolves to nothing. THIS
 *     is the defect — a graph built before a refactor still naming a deleted file.
 *   - empty: graphify emitted a node with no source_file at all.
 *   - local: an existing but untracked path. It is ignored rather than reported, so
 *     machine-local Graphify memory/cache/output can never enter committed metadata.
 *
 * Both predicates are injected rather than imported so fixtures can describe a virtual
 * tracked tree plus ignored local output independently.
 */
export function classifyPath(
  raw: string | null | undefined,
  tracked: (p: string) => boolean,
  exists: (p: string) => boolean = tracked,
): ClassifiedPath {
  if (raw === null || raw === undefined || raw.trim() === "") {
    return { cls: "empty", raw: raw ?? "" };
  }

  const candidates = [raw, ...[...RESOLVE_EXTENSIONS, ...RESOLVE_INDEXES].map((suffix) => raw + suffix)];
  for (const candidate of candidates) {
    if (tracked(candidate)) return { cls: "internal", path: candidate, raw };
  }

  // Existing-but-untracked files are local checkout state, not stale Axon sources.
  // Drop them without retaining the raw filename in any reported bucket.
  if (candidates.some(exists)) return { cls: "local", raw };

  // Unresolvable. A `$`-alias is always foreign (SvelteKit's `$app/*`, `$lib/*` style).
  // Otherwise: under a known root means it should have existed (stale); anywhere else
  // means it was never ours (a bare npm/crate specifier).
  if (raw.startsWith("$")) return { cls: "external", raw };
  const root = raw.split("/")[0];
  return INTERNAL_ROOTS.includes(root) ? { cls: "stale", raw } : { cls: "external", raw };
}

/**
 * Map an internal path to the unit that owns it.
 *
 * `dashboard` is the spine shell and owns its whole directory, so it has no `<name>`
 * segment to read — it IS the unit. `tools` and `schemas` are the same shape. The
 * three-noun model (CONTRIBUTING.md#three-architectural-nouns) is what decides these are units at all: a Pack is as much
 * a thing that can couple to a capability as another capability is.
 */
export function unitForPath(path: string): Unit | null {
  const parts = path.split("/");
  const [root, second] = parts;
  switch (root) {
    case "capabilities":
      return second ? { name: second, kind: "capability" } : null;
    case "libs":
      return second ? { name: second, kind: "lib" } : null;
    case "Packs":
      return second ? { name: second, kind: "pack" } : null;
    case "dashboard":
    case "tools":
    case "schemas":
      return { name: root, kind: "spine" };
    default:
      return null;
  }
}

export interface UnitRollup {
  name: string;
  kind: UnitKind;
  /** Distinct canonical files — deduplicated, so a specifier and its target count once. */
  files: number;
  /** graphify nodes attributed to this unit, before file dedup. */
  nodes: number;
}

export interface Buckets {
  /** Foreign module specifiers, e.g. `svelte`, `maplibre-gl`, `$app/state`. */
  external: number;
  /** Internal-looking paths that resolve to nothing — the staleness signal. */
  stale: string[];
  /** Nodes graphify emitted with no source_file. */
  empty: number;
  /** Resolved internal paths under no known unit root, e.g. a root-level README. */
  unmatched: string[];
}

export interface Rollup {
  units: UnitRollup[];
  buckets: Buckets;
  /** Nodes admitted through the public-safe boundary; local nodes are excluded. */
  admittedNodes: number;
  /** node id -> unit name, for edge attribution. Never emitted into self.json. */
  unitById: Map<string, Unit>;
}

export interface GraphNode {
  id: string;
  source_file?: string | null;
  /** graphify's label. For an import target it is the specifier as written, e.g. `../core/ops.ts`. */
  label?: string | null;
}

/**
 * An import target graphify resolved against the importing file's directory to a path that does
 * not exist: a relative specifier in a minified bundle, a build script or a tsconfig that names a
 * file this tree never had. It is a dangling reference written in a tracked file, not a source a
 * refactor deleted, so it is counted with the foreign specifiers and never reported as stale.
 * Measured 2026-09-26: all 10 such nodes came from five files (a minified test mock, two pi-package
 * build scripts, two tsconfig.json), and a fresh AST-only rebuild still produced them.
 */
export function isUnresolvedRelativeImport(node: GraphNode): boolean {
  const label = node.label ?? "";
  return label.startsWith("./") || label.startsWith("../");
}

/**
 * Roll every graph node up to its unit.
 *
 * Node counts and file counts are deliberately both reported: they differ by exactly
 * the double-counting graphify introduces when it emits an import specifier and its
 * target file as two nodes, and seeing both numbers is how that stays visible instead
 * of being silently absorbed.
 */
export function rollUp(
  nodes: GraphNode[],
  tracked: (p: string) => boolean,
  exists: (p: string) => boolean = tracked,
): Rollup {
  const filesByUnit = new Map<string, Set<string>>();
  const nodesByUnit = new Map<string, number>();
  const kindByUnit = new Map<string, UnitKind>();
  const unitById = new Map<string, Unit>();
  const buckets: Buckets = { external: 0, stale: [], empty: 0, unmatched: [] };
  const staleSeen = new Set<string>();
  const unmatchedSeen = new Set<string>();
  let admittedNodes = 0;

  for (const node of nodes) {
    const c = classifyPath(node.source_file, tracked, exists);
    if (c.cls === "local") continue;
    admittedNodes++;
    if (c.cls === "empty") {
      buckets.empty += 1;
      continue;
    }
    if (c.cls === "external") {
      buckets.external += 1;
      continue;
    }
    if (c.cls === "stale" && isUnresolvedRelativeImport(node)) {
      buckets.external += 1;
      continue;
    }
    if (c.cls === "stale") {
      if (!staleSeen.has(c.raw)) {
        staleSeen.add(c.raw);
        buckets.stale.push(c.raw);
      }
      continue;
    }

    const path = c.path!;
    const unit = unitForPath(path);
    if (!unit) {
      if (!unmatchedSeen.has(path)) {
        unmatchedSeen.add(path);
        buckets.unmatched.push(path);
      }
      continue;
    }

    unitById.set(node.id, unit);
    kindByUnit.set(unit.name, unit.kind);
    nodesByUnit.set(unit.name, (nodesByUnit.get(unit.name) ?? 0) + 1);
    let files = filesByUnit.get(unit.name);
    if (!files) {
      files = new Set();
      filesByUnit.set(unit.name, files);
    }
    files.add(path);
  }

  const units: UnitRollup[] = [...filesByUnit.keys()]
    .sort()
    .map((name) => ({
      name,
      kind: kindByUnit.get(name)!,
      files: filesByUnit.get(name)!.size,
      nodes: nodesByUnit.get(name) ?? 0,
    }));

  buckets.stale.sort();
  buckets.unmatched.sort();
  return { units, buckets, admittedNodes, unitById };
}

/**
 * How one unit's code reaches into another's, as declared in ground truth.
 *
 * Axon does not couple units by depending on published crates. A capability pulls a lib
 * in by path — `#[path = "../../../libs/sjel-config/src/lib.rs"] mod sjel_config;` in a
 * source file, `sjel-config = { path = "../../libs/sjel-config" }` in its Cargo.toml.
 * Both are literal strings in tracked files, which makes them exact: there is nothing to
 * infer and no graph to trust. This is the ground truth that replaced graphify's import
 * edges after those were shown to be both false-positive and false-negative here (see the
 * ISA changelog).
 */
export interface SourceCoupling {
  from: string;
  to: string;
  kind: "rust-path" | "cargo-dep";
  /** The file the evidence lives in. */
  file: string;
  /** The literal string that proves it. */
  evidence: string;
}

/** Normalize a POSIX-ish relative path, resolving `.` and `..` segments. */
function normalizeRelative(base: string, rel: string): string {
  const stack: string[] = base.split("/").filter((s) => s !== "" && s !== ".");
  for (const seg of rel.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") stack.pop();
    else stack.push(seg);
  }
  return stack.join("/");
}

/**
 * Rust `#[path = "…"]` includes that resolve into a different unit.
 *
 * `file` is the repo-relative path of the source file, so the attribute's relative path
 * resolves against its directory. An include that stays inside the same unit is not
 * coupling and is dropped.
 */
export function couplingFromRustPath(file: string, text: string): SourceCoupling[] {
  const self = unitForPath(file);
  if (!self) return [];
  const dir = file.split("/").slice(0, -1).join("/");
  const out: SourceCoupling[] = [];
  for (const m of text.matchAll(/#\[path\s*=\s*"([^"]+)"\]/g)) {
    const target = normalizeRelative(dir, m[1]);
    const unit = unitForPath(target);
    if (!unit || unit.name === self.name) continue;
    out.push({ from: self.name, to: unit.name, kind: "rust-path", file, evidence: m[0] });
  }
  return out;
}

/**
 * Cargo path dependencies naming another unit, from any Cargo.toml.
 *
 * `file` is the repo-relative path of the manifest, so each `path = "…"` resolves against
 * its directory. Matches every `path = "…"` in the file — dependencies, dev-dependencies
 * and build-dependencies all express a real compile-time reach, and treating them alike
 * keeps this from silently missing a fourth table someone adds later. A registry
 * dependency carries no `path` and never matches; a path that stays inside the same unit
 * is not coupling and is dropped.
 */
export function couplingFromCargo(file: string, text: string): SourceCoupling[] {
  const self = unitForPath(file);
  if (!self) return [];
  const dir = file.split("/").slice(0, -1).join("/");
  const out = new Map<string, SourceCoupling>();
  for (const m of text.matchAll(/\bpath\s*=\s*"([^"]+)"/g)) {
    const unit = unitForPath(normalizeRelative(dir, m[1]));
    if (!unit || unit.name === self.name) continue;
    const key = `${self.name}->${unit.name}`;
    if (!out.has(key)) {
      out.set(key, { from: self.name, to: unit.name, kind: "cargo-dep", file, evidence: m[0] });
    }
  }
  return [...out.values()];
}

/**
 * Merge per-file coupling into a deterministic, deduplicated unit-level map.
 *
 * Both evidence kinds are kept per pair rather than collapsed to a boolean: a pair backed
 * by `rust-path` but NOT `cargo-dep` is a source include the manifest never declared, and
 * the reverse is a dependency nothing imports.
 */
export function mergeCoupling(edges: SourceCoupling[]): Array<{
  from: string;
  to: string;
  kinds: string[];
  evidence: string[];
}> {
  const acc = new Map<string, { kinds: Set<string>; evidence: Set<string> }>();
  for (const e of edges) {
    const key = `${e.from} ${e.to}`;
    let entry = acc.get(key);
    if (!entry) {
      entry = { kinds: new Set(), evidence: new Set() };
      acc.set(key, entry);
    }
    entry.kinds.add(e.kind);
    entry.evidence.add(e.file);
  }
  return [...acc.entries()]
    .map(([key, v]) => {
      const [from, to] = key.split(" ");
      return { from, to, kinds: [...v.kinds].sort(), evidence: [...v.evidence].sort() };
    })
    .sort((a, b) => (a.from === b.from ? a.to.localeCompare(b.to) : a.from.localeCompare(b.from)));
}

// generateWouldDropCode and generateWouldBakeStaleGraph lived here until 2026-09-29. Both guarded
// `tools/self generate` against writing an artifact whose `code` layer had been dropped on a
// graphless machine or rolled up from a stale graph, and both existed only because that layer was
// committed. It is fused on read now (tools/self.ts, "committed vs fused"), so what is left is
// derived from tracked files alone and there is nothing left to guard.
