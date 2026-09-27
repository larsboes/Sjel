// tools/self.ts — Axon's self-model: what this repo contains, what is wired to what,
// where each upstream stands, and how much code each unit holds. One committed artifact
// (self.json) plus a query surface over it.
//
// Two consumers, by design. An agent starting work here reads self.json whole — it is
// deliberately kept small enough for that — instead of rediscovering the same structure
// by hand every session. A human runs `tools/self status` or `tools/self explain comms`.
// The dashboard panel (Axon#66) is a third view over the same file, not a second source.
//
// What is committed vs fused on read is the load-bearing distinction:
//
//   committed   structure, provenance, code rollup, coupling map — all derived from
//               tracked files, so two runs on an unchanged tree are byte-identical and
//               the artifact survives a fresh clone.
//   fused       live process health (axon-status owns it) and open issue counts (the
//               tracker owns them). Copying either into a committed file would give one
//               fact two homes and make the file lie the moment a process stops or an
//               issue is triaged.
//
// TypeScript rather than bash under the tools-doctor-typescript-not-bash precedent: this
// parses upstreams.toml via Bun.TOML and does set arithmetic over a 3,965-node graph,
// neither of which tools/lib/toml.sh's single-line grep/sed contract can express. The
// pure logic lives in tools/lib/self-model.ts and is tested by tools/self.test.ts; this
// file owns all I/O.
//
//   tools/self generate          # regenerate self.json from the working tree
//   tools/self status            # one row per unit (add --online for open issue counts)
//   tools/self explain <unit>    # wiring, code size, provenance for one unit
//   tools/self coupling          # what is compiled into what, with evidence
//   tools/self check             # is the committed self.json still current?
//   tools/self -h                # this help
//
// Exit 0 = fine, 1 = stale (check), a stale code graph (generate), or an unknown unit
// (explain).

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import {
  couplingFromCargo,
  couplingFromRustPath,
  generateWouldBakeStaleGraph,
  generateWouldDropCode,
  mergeCoupling,
  rollUp,
  type SourceCoupling,
} from "./lib/self-model.ts";

const HELP = `tools/self — Axon's self-model: structure, coupling, provenance, code size.

  tools/self generate          regenerate self.json from the working tree
  tools/self status            one row per unit (--online adds open issue counts)
  tools/self explain <unit>    wiring, code size, provenance for one unit
  tools/self coupling          what is compiled into what, with evidence
  tools/self check             is the committed self.json still current? (exit 1 if not)

  --json                       machine-readable output for status/explain/coupling
  --allow-stale                generate anyway when the code graph holds paths this tree
                               does not have. Prefer 'graphify update .' first.
  --against <path>             check this tree against that artifact instead of self.json
`;

const SJEL_ROOT = resolve(import.meta.dir, "..");
const SELF_JSON = `${SJEL_ROOT}/self.json`;

/** The artifact's shape. Bump `schema` when a consumer would need to care. */
interface SelfModel {
  schema: 1;
  /** Deliberately NOT a timestamp: a generated-at field would make every run differ. */
  generator: string;
  units: Array<{
    name: string;
    kind: string;
    /** Present for anything with a service.toml. */
    service?: { kind: string; port?: string; requires: string[]; image?: string };
    code?: { files: number; nodes: number };
  }>;
  /** Compile-time coupling: what is pulled into what. Distinct from service `requires`. */
  coupling: Array<{ from: string; to: string; kinds: string[]; evidence: string[] }>;
  /** url/verdict/license/why is the whole register; `pin` was deleted 2026-09-02 (Q77). */
  upstreams: Array<{ name: string; verdict: string }>;
  /** Honest accounting of what the code graph could not attribute. */
  graph: { present: boolean; nodes: number; external: number; stale: string[]; unmatched: string[] };
}

function readText(path: string): string | null {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return null;
  }
}

/** What a capability's own manifest declares about its service, before any machine touches it. */
interface DeclaredService {
  kind: string;
  requires: string[];
  port?: string;
  image?: string;
}

/**
 * Declared service facts, read from `capabilities/<name>/service.toml` in this checkout.
 *
 * NOT from `tools/capability.sh registry`, which is what this used to do. That registry merges the
 * running machine's `machine.toml` overrides -- `[capability.<name>] ports` among them -- and hard
 * fails without one. self.json is a tracked artifact, so both consequences were defects: a port in
 * it was whatever the generating machine resolved rather than what the repository declares, so two
 * machines produced different files from the same commit; and `tools/self check` could not run
 * anywhere without an overlay, which kept it out of CI's repo-gates job and left it a doctor-only
 * check. Worse quietly: readRegistry() returned [] when the registry failed, so a checkout without
 * an overlay dropped every service block and `check` reported the artifact stale.
 *
 * `tools/generate-architecture.sh` never had this problem because it reads each service.toml
 * directly, which is why ARCHITECTURE.md has always been reproducible from a fresh clone. This is
 * the same choice, in the language self.ts is written in: Bun.TOML under the documented exception
 * tools/doctor.ts already takes, rather than a per-manifest shell-out to tools/lib/toml.sh.
 *
 * Overlay capabilities are absent by construction now instead of by a scope filter: they live in
 * the overlay's own tree, which this never reads. A capability name is itself a fact about a
 * private deployment (Axon#225).
 */
function readDeclaredServices(trackedPaths: Set<string>): Map<string, DeclaredService> {
  const out = new Map<string, DeclaredService>();
  // Discovered from the tracked tree, not from a directory list. capabilities/<name>/service.toml
  // is where most of them live, but the dashboard owns one at the repository root -- a hardcoded
  // "capabilities" scan dropped its service block, and a hardcoded list of spine names would drop
  // the next one the same way.
  for (const path of trackedPaths) {
    if (!path.endsWith("/service.toml")) continue;
    const segments = path.split("/");
    const name = segments.length === 3 && segments[0] === "capabilities" ? segments[1]
      : segments.length === 2 ? segments[0]
      : "";
    if (!name) continue;
    let parsed: Record<string, unknown>;
    try {
      parsed = Bun.TOML.parse(readFileSync(`${SJEL_ROOT}/${path}`, "utf8")) as Record<string, unknown>;
    } catch {
      continue; // A manifest that does not parse is tools/doctor's finding, not a reason to abort.
    }
    const requires = parsed.requires;
    out.set(name, {
      // service-runner.sh treats an absent kind as a container; the manifest and the model have to
      // agree on that default or the two disagree about capabilities that never write the line.
      kind: parsed.kind == null ? "container" : String(parsed.kind),
      requires: Array.isArray(requires) ? requires.map(String) : [],
      port: parsed.port == null ? undefined : String(parsed.port),
      image: parsed.image == null ? undefined : String(parsed.image),
    });
  }
  return out;
}

function readUpstreams(): SelfModel["upstreams"] {
  const text = readText(`${SJEL_ROOT}/upstreams.toml`);
  if (!text) return [];
  const parsed = Bun.TOML.parse(text) as Record<string, { verdict?: string }>;
  return Object.entries(parsed)
    .map(([name, v]) => ({ name, verdict: v?.verdict ?? "" }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Walk tracked sources for ground-truth coupling.
 *
 * `git ls-files` rather than a filesystem glob: an untracked scratch file is not part of
 * what this repo IS, and including it would make the committed artifact depend on
 * whatever happens to be lying in the working tree.
 */
function readCoupling(): SourceCoupling[] {
  const proc = Bun.spawnSync({
    cmd: ["git", "-C", SJEL_ROOT, "ls-files", "*.rs", "Cargo.toml", "*/Cargo.toml"],
    stdout: "pipe",
  });
  const files = proc.stdout.toString().split("\n").filter(Boolean);
  const edges: SourceCoupling[] = [];
  for (const file of files) {
    const text = readText(`${SJEL_ROOT}/${file}`);
    if (text === null) continue;
    if (file.endsWith(".rs")) edges.push(...couplingFromRustPath(file, text));
    if (file.endsWith("Cargo.toml")) edges.push(...couplingFromCargo(file, text));
  }
  return edges;
}

/** Public-safe graph input boundary: only Git-tracked paths may become internal metadata. */
function readTrackedPaths(): Set<string> {
  const proc = Bun.spawnSync({
    cmd: ["git", "-C", SJEL_ROOT, "ls-files", "-z"],
    stdout: "pipe",
  });
  if (proc.exitCode !== 0) return new Set();
  return new Set(proc.stdout.toString().split("\0").filter(Boolean));
}

function build(): SelfModel {
  // SJEL_SELF_GRAPH is a test seam, and it exists because the obvious way to test the
  // stale-graph refusal is to plant a graph — and on this machine graphify-out/ holds a
  // real 13,747-node graph that took a run to build. A test that wrote there to prove a
  // refusal would destroy the thing it was protecting.
  const graphText = readText(process.env.SJEL_SELF_GRAPH || `${SJEL_ROOT}/graphify-out/graph.json`);
  const trackedPaths = readTrackedPaths();
  const declaredServices = readDeclaredServices(trackedPaths);
  const tracked = (p: string) => trackedPaths.has(p);
  const exists = (p: string) => existsSync(`${SJEL_ROOT}/${p}`);

  let rollupUnits: Array<{ name: string; kind: string; files: number; nodes: number }> = [];
  let graph: SelfModel["graph"] = { present: false, nodes: 0, external: 0, stale: [], unmatched: [] };
  if (graphText) {
    const parsed = JSON.parse(graphText);
    const r = rollUp(parsed.nodes ?? [], tracked, exists);
    rollupUnits = r.units;
    graph = {
      present: true,
      nodes: r.admittedNodes,
      external: r.buckets.external,
      stale: r.buckets.stale,
      unmatched: r.buckets.unmatched,
    };
  }

  // The unit inventory comes from the tracked tree, never from the code graph.
  //
  // Deriving it from the graph made the whole artifact depend on git-ignored
  // graphify-out/: on a fresh clone the unit list silently collapsed from 31 to the ~12
  // capabilities that happen to own a service.toml, dropping every Pack and lib. What
  // Axon *contains* is a fact about tracked files, so it is read from them; the graph
  // only ever contributes `code` counts on top.
  const kindByUnit = new Map<string, string>();
  const addDirs = (parent: string, kind: string) => {
    try {
      for (const name of readdirSync(`${SJEL_ROOT}/${parent}`, { withFileTypes: true })) {
        if (name.isDirectory()) kindByUnit.set(name.name, kind);
      }
    } catch {
      // A missing top-level directory is not an error: a minimal install has no Packs.
    }
  };
  addDirs("capabilities", "capability");
  addDirs("libs", "lib");
  addDirs("Packs", "pack");
  for (const spine of ["dashboard", "tools", "schemas"]) {
    if (existsSync(`${SJEL_ROOT}/${spine}`)) kindByUnit.set(spine, "spine");
  }

  const codeByUnit = new Map(rollupUnits.map((u) => [u.name, u]));
  const names = new Set<string>(kindByUnit.keys());
  for (const name of declaredServices.keys()) names.add(name);

  const units: SelfModel["units"] = [...names].sort().map((name) => {
    const declared = declaredServices.get(name);
    const code = codeByUnit.get(name);
    const kind = kindByUnit.get(name) ?? (declared ? "capability" : "unknown");
    const out: SelfModel["units"][number] = { name, kind };
    if (declared) {
      out.service = { kind: declared.kind, requires: declared.requires };
      if (declared.port) out.service.port = declared.port;
      if (declared.image) out.service.image = declared.image;
    }
    if (code) out.code = { files: code.files, nodes: code.nodes };
    return out;
  });

  return {
    schema: 1,
    generator: "tools/self.ts",
    units,
    coupling: mergeCoupling(readCoupling()),
    upstreams: readUpstreams(),
    graph,
  };
}

/** Stable stringify via sorted construction above — key order is insertion order. */
function serialize(model: SelfModel): string {
  return JSON.stringify(model, null, 2) + "\n";
}

function loadCommitted(): SelfModel | null {
  const text = readText(SELF_JSON);
  return text ? (JSON.parse(text) as SelfModel) : null;
}

/**
 * Drop the layers that only a machine with a built code graph can reproduce.
 *
 * `self.json` is committed, but its per-unit `code` counts and its `graph` block are
 * derived from `graphify-out/`, which is git-ignored because graphify slugifies node ids
 * from the absolute scan path. A fresh clone — CI, or another machine before it runs
 * `tools/graphify.sh` — therefore cannot regenerate those two layers, and a naive
 * full-text comparison reports the file stale when nothing is actually wrong.
 *
 * That is the same defect class this run gated against elsewhere: a committed artifact
 * depending on an input others cannot see. The honest resolution is not to hide the
 * mismatch but to narrow the claim — compare everything reproducible here, and say which
 * scope was checked. Structure, coupling and provenance all come from tracked files, so
 * they are gateable anywhere; the code layer is gated wherever a graph exists.
 */
function stripLocalLayers(model: SelfModel): Omit<SelfModel, "graph"> & { units: SelfUnitLike[] } {
  const { graph: _graph, ...rest } = model;
  return { ...rest, units: model.units.map(({ code: _code, ...u }) => u) };
}
type SelfUnitLike = Omit<SelfModel["units"][number], "code">;

/** Open issues per unit, joined on the `<unit>:` title prefix the tracker already uses. */
function openIssuesByUnit(): { counts: Map<string, number>; unmatched: number } | null {
  // No --repo: gh resolves it from this checkout's remote, the same way the
  // `git -C SJEL_ROOT` calls above resolve theirs. It was hardcoded to one
  // owner/name, which is a deployment fact in public code and would have gone
  // on querying that name after a rename — answering from whatever repository
  // happened to hold it rather than from this one.
  const proc = Bun.spawnSync({
    cmd: ["gh", "issue", "list", "--state", "open", "--limit", "200", "--json", "title"],
    cwd: SJEL_ROOT,
    stdout: "pipe",
    stderr: "pipe",
  });
  if (proc.exitCode !== 0) return null;
  let rows: Array<{ title: string }>;
  try {
    rows = JSON.parse(proc.stdout.toString());
  } catch {
    return null;
  }
  const counts = new Map<string, number>();
  let unmatched = 0;
  for (const { title } of rows) {
    const m = title.match(/^([A-Za-z0-9._-]+):/);
    if (m) counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
    else unmatched += 1;
  }
  return { counts, unmatched };
}

const args = process.argv.slice(2);
const wantJson = args.includes("--json");
const online = args.includes("--online");
const cmd = args.find((a) => !a.startsWith("-")) ?? "status";

if (args.includes("-h") || args.includes("--help")) {
  console.log(HELP);
  process.exit(0);
}

if (cmd === "generate") {
  const fresh = build();
  // Refuse rather than write a gutted artifact. The `code` layer is derived from
  // graphify-out/graph.json, which is git-ignored and machine-local; on a fresh clone or any
  // machine that has not run the graph, every unit's counts are simply absent. Writing that out
  // silently removed 181 lines from the committed file, and `tools/self check` would then pass on
  // the result for exactly the same reason it narrows its claim -- so nothing downstream objects
  // (#35). Carrying the committed numbers forward instead is not an option: they would describe a
  // tree that no longer exists, which is a different lie.
  const committed = loadCommitted();
  if (generateWouldDropCode(fresh.graph.present, committed?.units ?? null)) {
    console.error(
      "self.json carries per-unit code counts, and there is no code graph here to reproduce them.",
    );
    console.error("Writing now would drop that layer for everyone. Build the graph first:");
    console.error("  tools/graphify.sh");
    process.exit(1);
  }
  // Refuse to write a measurement of a tree that no longer exists. `graph.stale` is not a
  // field to record, it is a reason not to write: every per-unit `code` count in this run was
  // rolled up from the same graph. See generateWouldBakeStaleGraph.
  if (generateWouldBakeStaleGraph(fresh.graph.stale) && !args.includes("--allow-stale")) {
    console.error(
      `the code graph still holds ${fresh.graph.stale.length} path(s) this tree does not have:`,
    );
    for (const path of fresh.graph.stale.slice(0, 10)) console.error(`  ${path}`);
    if (fresh.graph.stale.length > 10) {
      console.error(`  ... and ${fresh.graph.stale.length - 10} more`);
    }
    console.error("Every per-unit code count here was rolled up from that graph, so writing now");
    console.error("records the staleness instead of fixing it. Rebuild the graph first:");
    console.error("  graphify update .        # AST-only, no API cost");
    console.error("Then run generate again. --allow-stale writes it anyway.");
    process.exit(1);
  }
  if (generateWouldBakeStaleGraph(fresh.graph.stale)) {
    console.error(
      `WARNING: --allow-stale — baking ${fresh.graph.stale.length} stale graph path(s) into self.json.`,
    );
  }
  const out = serialize(fresh);
  await Bun.write(SELF_JSON, out);
  console.log(`wrote ${SELF_JSON} (${out.length} bytes)`);
  process.exit(0);
}

/**
 * Show what actually differs, not only that something does.
 *
 * `check` reported "self.json is stale" and stopped, which leaves the reader with a whole
 * artifact to eyeball and no idea whether a port moved or the entire code layer vanished.
 * The same problem already has an answer in this repository: tools/check-architecture-fresh.sh
 * regenerates into a scratch file and prints `diff` of the two. This is that, in TypeScript —
 * `diff -u` rather than a diff engine written here, with `-L` for the labels because both BSD
 * and GNU diff accept it.
 *
 * Capped, because a first generate on a machine with a fresh graph can differ by thousands of
 * lines and a terminal full of JSON is the same non-answer as no diff at all.
 */
function printDrift(committedText: string, freshText: string, cap = 120): void {
  const dir = mkdtempSync(`${tmpdir()}/axon-self-check.`);
  writeFileSync(`${dir}/committed`, committedText);
  writeFileSync(`${dir}/fresh`, freshText);
  const proc = Bun.spawnSync({
    cmd: [
      "diff", "-u",
      "-L", "self.json (committed)",
      "-L", "self.json (this tree)",
      `${dir}/committed`, `${dir}/fresh`,
    ],
    stdout: "pipe",
    stderr: "pipe",
  });
  const lines = proc.stdout.toString().split("\n").filter((l) => l.length > 0);
  if (lines.length === 0) {
    // diff found nothing while the string comparison did, or diff is absent. Say which
    // rather than printing an empty block that reads as "no differences".
    console.error(`  (could not render a diff: ${proc.stderr.toString().trim() || "diff produced no output"})`);
    return;
  }
  for (const line of lines.slice(0, cap)) console.error(`  ${line}`);
  if (lines.length > cap) console.error(`  ... ${lines.length - cap} more diff lines`);
}

if (cmd === "check") {
  // --against <path> compares this tree against an artifact that is not the committed one.
  // It is what lets tools/self.test.sh watch the stale path produce a real diff without
  // editing the tracked self.json, and it answers "is the file on that branch current?"
  // without a checkout.
  const againstAt = args.indexOf("--against");
  const comparePath = againstAt === -1 ? SELF_JSON : args[againstAt + 1];
  if (!comparePath) {
    console.error("tools/self check --against needs a path");
    process.exit(1);
  }
  const committedText = readText(comparePath);
  if (!committedText) {
    console.error(
      comparePath === SELF_JSON
        ? "self.json is missing. Run: tools/self generate"
        : `cannot read ${comparePath}`,
    );
    process.exit(1);
  }
  const fresh = build();
  // Full comparison only where a code graph exists to reproduce those layers; elsewhere
  // (a fresh clone, CI) narrow the claim to the tracked-file layers rather than reporting
  // a mismatch nobody can fix without running graphify. See stripLocalLayers.
  const scope = fresh.graph.present ? "full" : "structure, coupling and provenance";
  const left = fresh.graph.present ? serialize(fresh) : JSON.stringify(stripLocalLayers(fresh), null, 2);
  const right = fresh.graph.present
    ? committedText
    : JSON.stringify(stripLocalLayers(JSON.parse(committedText) as SelfModel), null, 2);

  if (left === right) {
    console.log(
      fresh.graph.present
        ? "self.json is current."
        : `self.json is current (${scope}; no code graph here, so per-unit code counts were not compared).`,
    );
    process.exit(0);
  }
  console.error(`self.json is stale (${scope} differ). Run: tools/self generate`);
  printDrift(right, left);
  process.exit(1);
}

const model = loadCommitted() ?? build();

if (cmd === "coupling") {
  if (wantJson) {
    console.log(JSON.stringify(model.coupling, null, 2));
    process.exit(0);
  }
  console.log(`Compile-time coupling — what is pulled into what (${model.coupling.length} pairs).`);
  console.log("Distinct from service `requires`, which is what must be RUNNING.\n");
  for (const e of model.coupling) {
    console.log(`  ${e.from.padEnd(14)} -> ${e.to.padEnd(14)} [${e.kinds.join("+")}]`);
  }
  process.exit(0);
}

if (cmd === "explain") {
  const name = args.find((a) => !a.startsWith("-") && a !== "explain");
  const unit = model.units.find((u) => u.name === name);
  if (!unit) {
    console.error(`unknown unit '${name}'. Known: ${model.units.map((u) => u.name).join(", ")}`);
    process.exit(1);
  }
  if (wantJson) {
    console.log(JSON.stringify(unit, null, 2));
    process.exit(0);
  }
  console.log(`${unit.name} (${unit.kind})`);
  if (unit.code) console.log(`  code       ${unit.code.files} files, ${unit.code.nodes} graph nodes`);
  if (unit.service) {
    console.log(`  service    kind=${unit.service.kind}${unit.service.port ? ` port=${unit.service.port}` : ""}`);
    console.log(`  requires   ${unit.service.requires.length ? unit.service.requires.join(", ") : "—"} (must be running)`);
  }
  const out = model.coupling.filter((c) => c.from === unit.name).map((c) => c.to);
  const inc = model.coupling.filter((c) => c.to === unit.name).map((c) => c.from);
  console.log(`  compiles in ${out.length ? out.join(", ") : "—"}`);
  console.log(`  used by     ${inc.length ? inc.join(", ") : "—"}`);
  process.exit(0);
}

// Default: status
const work = online ? openIssuesByUnit() : null;
if (wantJson) {
  console.log(JSON.stringify({ ...model, work: work ? Object.fromEntries(work.counts) : null }, null, 2));
  process.exit(0);
}
console.log(`Axon self-model — ${model.units.length} units, ${model.coupling.length} coupling pairs`);
console.log(
  model.graph.present
    ? `Code graph: ${model.graph.nodes} nodes, ${model.graph.external} external, ${model.graph.stale.length} stale\n`
    : "Code graph: absent (run tools/graphify.sh)\n",
);
const header = `  ${"unit".padEnd(18)}${"kind".padEnd(12)}${"files".padStart(6)}${"port".padStart(7)}${"requires".padStart(12)}`;
console.log(header + (work ? "   open" : ""));
for (const u of model.units) {
  let row = `  ${u.name.padEnd(18)}${u.kind.padEnd(12)}`;
  row += String(u.code?.files ?? "—").padStart(6);
  row += String(u.service?.port ?? "—").padStart(7);
  row += String(u.service?.requires.length ? u.service.requires.join(",") : "—").padStart(12);
  if (work) row += String(work.counts.get(u.name) ?? 0).padStart(7);
  console.log(row);
}
if (work) console.log(`\n  ${work.unmatched} open issues match no unit prefix.`);
if (model.graph.stale.length) {
  console.log(`\n  ⚠ ${model.graph.stale.length} graph paths no longer exist — run tools/graphify.sh`);
}
