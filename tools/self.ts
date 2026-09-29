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
//   committed   structure, provenance and coupling — all derived from tracked files, so two
//               runs on an unchanged tree are byte-identical and the artifact survives a
//               fresh clone.
//   fused       per-unit code counts and the graph accounting (both rolled up from
//               graphify-out/, which is git-ignored and machine-local), live process health
//               (sjel-status owns it) and open issue counts (the tracker owns them). Copying
//               any of them into a committed file gives one fact two homes and makes the file
//               lie the moment a process stops, an issue is triaged, or a graph goes stale.
//
// The code rollup sat in the wrong column until 2026-09-29, and that is why this distinction is
// load-bearing rather than decorative. It fails the committed column's own test — a fresh clone
// cannot reproduce it — and tools/generate-site.ts already refused to publish it for exactly that
// reason. Committing it anyway made the artifact checkable-but-unfixable off a graphful machine:
// `check` narrowed its comparison to the tracked-file layers and reported the drift, while
// `generate` refused to write because writing would drop the counts (#35). CI is that machine, so
// the one drift CI reported was the one drift nobody could repair, and main sat red while eleven
// armed Dependabot pull requests queued behind it. The counts are fused on read now: `status` and
// `explain` still show them where a graph exists, and no downstream has to reproduce them.
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
// Exit 0 = fine, 1 = stale (check) or an unknown unit (explain).

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import {
  couplingFromCargo,
  couplingFromRustPath,
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
  --out <path>                 generate writes there instead of self.json
  --against <path>             check this tree against that artifact instead of self.json

Code size is fused on read from graphify-out/, which is git-ignored: status and explain show it
where a graph exists, and self.json never carries it. Run tools/graphify.sh to build one.
`;

const SJEL_ROOT = resolve(import.meta.dir, "..");
const SELF_JSON = `${SJEL_ROOT}/self.json`;

/** The artifact's shape. Bump `schema` when a consumer would need to care. */
interface SelfModel {
  schema: 2;
  /** Deliberately NOT a timestamp: a generated-at field would make every run differ. */
  generator: string;
  units: Array<{
    name: string;
    kind: string;
    /** Present for anything with a service.toml. */
    service?: { kind: string; port?: string; requires: string[]; image?: string };
  }>;
  /** Compile-time coupling: what is pulled into what. Distinct from service `requires`. */
  coupling: Array<{ from: string; to: string; kinds: string[]; evidence: string[] }>;
  /** url/verdict/license/why is the whole register; `pin` was deleted 2026-09-02 (Q77). */
  upstreams: Array<{ name: string; verdict: string }>;
}

/**
 * Per-unit code counts and the graph accounting. Fused on read, never committed — see the
 * header. Absent whenever this machine has not built graphify-out/graph.json.
 */
interface CodeLayer {
  byUnit: Map<string, { files: number; nodes: number }>;
  nodes: number;
  external: number;
  /** Paths the graph still holds a node for and the tree no longer has. */
  stale: string[];
  unmatched: string[];
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

/**
 * The per-unit code counts and the graph accounting, read from graphify-out/graph.json.
 *
 * SJEL_SELF_GRAPH is a test seam: the obvious way to probe this rollup is to plant a graph, and on
 * this machine graphify-out/ holds a real one that took a run to build, so a test that wrote there
 * to prove something would destroy the thing it was protecting.
 */
function readCodeLayer(trackedPaths: Set<string>): CodeLayer | null {
  const graphText = readText(process.env.SJEL_SELF_GRAPH || `${SJEL_ROOT}/graphify-out/graph.json`);
  if (!graphText) return null;
  const parsed = JSON.parse(graphText);
  const r = rollUp(
    parsed.nodes ?? [],
    (p) => trackedPaths.has(p),
    (p) => existsSync(`${SJEL_ROOT}/${p}`),
  );
  return {
    byUnit: new Map(r.units.map((u) => [u.name, { files: u.files, nodes: u.nodes }])),
    nodes: r.admittedNodes,
    external: r.buckets.external,
    stale: r.buckets.stale,
    unmatched: r.buckets.unmatched,
  };
}

function build(): SelfModel {
  const trackedPaths = readTrackedPaths();
  const declaredServices = readDeclaredServices(trackedPaths);

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

  const names = new Set<string>(kindByUnit.keys());
  for (const name of declaredServices.keys()) names.add(name);

  const units: SelfModel["units"] = [...names].sort().map((name) => {
    const declared = declaredServices.get(name);
    const kind = kindByUnit.get(name) ?? (declared ? "capability" : "unknown");
    const out: SelfModel["units"][number] = { name, kind };
    if (declared) {
      out.service = { kind: declared.kind, requires: declared.requires };
      if (declared.port) out.service.port = declared.port;
      if (declared.image) out.service.image = declared.image;
    }
    return out;
  });

  return {
    schema: 2,
    generator: "tools/self.ts",
    units,
    coupling: mergeCoupling(readCoupling()),
    upstreams: readUpstreams(),
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
  // No refusal path any more. Both of them guarded the `code` layer — one against dropping it on a
  // graphless machine, one against writing counts rolled up from a stale graph — and that layer is
  // fused on read now (see the header). What is left is derived from tracked files, so it is the
  // same on every machine and there is nothing to lose by writing it.
  //
  // --out exists so a test can watch a successful generate without writing the tracked artifact.
  const outAt = args.indexOf("--out");
  const target = outAt === -1 ? SELF_JSON : args[outAt + 1];
  if (!target) {
    console.error("tools/self generate --out needs a path");
    process.exit(1);
  }
  const out = serialize(build());
  await Bun.write(target, out);
  console.log(`wrote ${target} (${out.length} bytes)`);
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
  // One comparison, the same on every machine. It used to narrow its claim to the tracked-file
  // layers wherever no graph was present, and that is what made the drift CI reported the one
  // drift nobody could repair: the comparison said stale while `generate` refused to write.
  // Nothing compared here comes from graphify-out/ any more, so the narrowing has no reason to
  // exist — and a `check` that passes on a graphless machine is now a check that machine can act
  // on.
  const left = serialize(fresh);
  const right = committedText;

  if (left === right) {
    console.log("self.json is current.");
    process.exit(0);
  }
  console.error("self.json is stale. Run: tools/self generate");
  printDrift(right, left);
  process.exit(1);
}

const model = loadCommitted() ?? build();
// Fused on read: absent on any machine that has not built graphify-out/. Status and explain
// degrade to a dash rather than to a number nobody can check.
const code = readCodeLayer(readTrackedPaths());

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
  const counts = code?.byUnit.get(unit.name);
  if (wantJson) {
    console.log(JSON.stringify(counts ? { ...unit, code: counts } : unit, null, 2));
    process.exit(0);
  }
  console.log(`${unit.name} (${unit.kind})`);
  if (counts) console.log(`  code       ${counts.files} files, ${counts.nodes} graph nodes`);
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
  console.log(
    JSON.stringify(
      {
        ...model,
        code: code ? Object.fromEntries(code.byUnit) : null,
        work: work ? Object.fromEntries(work.counts) : null,
      },
      null,
      2,
    ),
  );
  process.exit(0);
}
console.log(`Axon self-model — ${model.units.length} units, ${model.coupling.length} coupling pairs`);
console.log(
  code
    ? `Code graph: ${code.nodes} nodes, ${code.external} external, ${code.stale.length} stale\n`
    : "Code graph: absent (run tools/graphify.sh)\n",
);
const header = `  ${"unit".padEnd(18)}${"kind".padEnd(12)}${"files".padStart(6)}${"port".padStart(7)}${"requires".padStart(12)}`;
console.log(header + (work ? "   open" : ""));
for (const u of model.units) {
  let row = `  ${u.name.padEnd(18)}${u.kind.padEnd(12)}`;
  row += String(code?.byUnit.get(u.name)?.files ?? "—").padStart(6);
  row += String(u.service?.port ?? "—").padStart(7);
  row += String(u.service?.requires.length ? u.service.requires.join(",") : "—").padStart(12);
  if (work) row += String(work.counts.get(u.name) ?? 0).padStart(7);
  console.log(row);
}
if (work) console.log(`\n  ${work.unmatched} open issues match no unit prefix.`);
if (code?.stale.length) {
  console.log(`\n  ⚠ ${code.stale.length} graph paths no longer exist — run tools/graphify.sh`);
}
