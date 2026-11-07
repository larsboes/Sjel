import { existsSync, readdirSync, readFileSync, realpathSync } from "node:fs";
import { dirname, join, normalize, relative, resolve } from "node:path";
import { firstHit, gpuiInputs } from "./ci-changes.ts";

export interface Package {
  name: string;
  dir: string;
  dependencies: string[];
  /** Repo-relative paths the package's sources name, so its tests may read them at run time. */
  reads?: string[];
}

export interface Impact {
  mode: "all" | "candidate";
  packages: string[];
  reasons: string[];
  skipped: string[];
}

/** Workspace-wide Rust inputs: a change here can change every package. */
function workspaceInput(file: string): boolean {
  return file === "Cargo.toml" || file === "Cargo.lock" || file === "rust-toolchain.toml" ||
    file === "clippy.toml" || file.startsWith(".cargo/");
}

/** True when `file` is `read` or lies under it; "." is the repository root. */
const under = (file: string, read: string) => read === "." || file === read || file.startsWith(`${read}/`);

/**
 * Packages whose build or tests a change can reach: the owner of each changed file, every
 * package whose sources name the file's path, and their reverse dependencies. A file that no
 * package owns or names selects nothing. A deleted Rust source with no owner widens to all.
 */
export function rustImpact(files: string[], packages: Package[], deleted: string[] = []): Impact {
  const all = packages.map((p) => p.name).sort();
  const selected = new Set<string>();
  const reasons: string[] = [];
  for (const file of new Set([...files, ...deleted])) {
    if (workspaceInput(file)) {
      return { mode: "all", packages: all, skipped: [], reasons: [`${file}: workspace-wide Rust input`] };
    }
    const owner = packages.filter((p) => file.startsWith(`${p.dir}/`)).sort((a, b) => b.dir.length - a.dir.length)[0];
    if (owner) {
      selected.add(owner.name);
      reasons.push(`${file}: owned by ${owner.name}`);
    }
    if (file.endsWith(".rs") && deleted.includes(file)) {
      return { mode: "all", packages: all, skipped: [], reasons: [`${file}: deleted Rust source with no owner`] };
    }
    const readers = packages.filter((p) => p.reads?.some((r) => under(file, r)));
    for (const p of readers) selected.add(p.name);
    reasons.push(readers.length ? `${file}: named by ${readers.map((p) => p.name).join(", ")}` : `${file}: named by no package`);
  }
  let changed = true;
  while (changed) {
    changed = false;
    for (const p of packages) {
      if (!selected.has(p.name) && p.dependencies.some((dep) => selected.has(dep))) {
        selected.add(p.name);
        reasons.push(`${p.name}: depends on a selected package`);
        changed = true;
      }
    }
  }
  return { mode: "candidate", packages: [...selected].sort(), skipped: all.filter((name) => !selected.has(name)), reasons };
}

const PATHLIKE = /^[A-Za-z0-9_.\-]+(\/[A-Za-z0-9_.\-]+)*\/?$/;

/**
 * Repo-relative paths a package's source can name. An upward literal ("../../docs") resolves
 * against its file and against the crate; any other literal counts when its first component
 * is a top-level entry of the repository. Joins built from parts still name the top-level
 * directory, which covers everything under it.
 */
export function namedPaths(source: string, file: string, crateDir: string, topLevel: Set<string>): string[] {
  const out = new Set<string>();
  for (const m of source.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)) {
    const literal = m[1].replace(/^\//, "");
    if (literal.startsWith("../")) {
      for (const base of [dirname(file), crateDir]) {
        const target = normalize(join(base, literal)).replace(/\/$/, "");
        if (target && !target.startsWith("..")) out.add(target);
      }
    } else if (PATHLIKE.test(literal)) {
      const path = literal.replace(/^\.\//, "").replace(/\/$/, "");
      if (topLevel.has(path.split("/")[0])) out.add(path);
    }
  }
  return [...out];
}

export interface FastCheck {
  name: string;
  cmd: string[];
  reason: string;
}

/**
 * Which test paths Bun's own discovery skips, from bunfig.toml's `[test] pathIgnorePatterns`.
 * A planned `bun test <file>` that Bun excludes does not run nothing quietly: Bun exits 1 with
 * "filters did not match any test files", so the gate fails on a file that is deliberately
 * outside its reach. Packages with their own runner (clipper's vitest) are asserted by their
 * own `check` script instead.
 */
export function bunDiscovers(root: string): (testPath: string) => boolean {
  const path = join(root, "bunfig.toml");
  if (!existsSync(path)) return () => true;
  const config = Bun.TOML.parse(readFileSync(path, "utf8")) as { test?: { pathIgnorePatterns?: string[] } };
  const ignored = config.test?.pathIgnorePatterns ?? [];
  const globs = ignored.map((p) => new Bun.Glob(p));
  return (testPath) => !globs.some((g) => g.match(testPath));
}

/** Cargo package arguments for an impact: `--workspace`, `-p` per package, or "" for none. */
export function testScope(impact: Impact): string {
  if (impact.mode === "all") return "--workspace";
  return impact.packages.map((p) => `-p ${p}`).join(" ");
}

/** The root workspace's packages, their in-workspace dependencies and the paths they name. */
export function rootPackages(root: string, manifest = "Cargo.toml"): Package[] {
  root = realpathSync(root);
  const metadata = JSON.parse(command(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", manifest], root));
  const members = new Set<string>(metadata.workspace_members);
  const workspace = metadata.packages.filter((p: any) => members.has(p.id));
  const namesByDir = new Map<string, string>(metadata.packages.map((p: any) => [dirname(p.manifest_path), p.name]));
  const topLevel = new Set(readdirSync(root).filter((n) => n !== ".git" && n !== "target"));
  return workspace.map((p: any) => {
    const dir = relative(root, dirname(p.manifest_path));
    const reads = new Set<string>();
    for (const e of readdirSync(join(root, dir), { recursive: true, withFileTypes: true })) {
      const path = relative(root, join(e.parentPath, e.name));
      if (!e.isFile() || !e.name.endsWith(".rs") || path.includes("/target/")) continue;
      for (const r of namedPaths(readFileSync(join(root, path), "utf8"), path, dir, topLevel)) reads.add(r);
    }
    return {
      name: p.name,
      dir,
      dependencies: p.dependencies.flatMap((d: any) => d.path ? [namesByDir.get(d.path) ?? d.name] : []),
      reads: [...reads].sort(),
    };
  });
}

/** Packages a cargo test log shows failing: a test binary that reported FAILED, or a crate that did not compile. */
export function failedPackages(log: string, targets: Map<string, string[]>): string[] {
  const failed = new Set<string>();
  let artifact = "";
  for (const line of log.split("\n")) {
    const running = line.match(/Running .* \((?:.*\/)?deps\/([A-Za-z0-9_]+)-[0-9a-f]+\)/);
    if (running) artifact = running[1];
    if (/test result: FAILED/.test(line) && artifact) for (const p of targets.get(artifact) ?? [`?${artifact}`]) failed.add(p);
    const compile = line.match(/error: could not compile `([^`]+)`/);
    if (compile) failed.add(compile[1]);
  }
  return [...failed].sort();
}

/** Target artifact name (dashes as underscores) to the packages that build it. */
export function targetMap(dir: string): Map<string, string[]> {
  const meta = JSON.parse(command(["cargo", "metadata", "--no-deps", "--format-version", "1"], dir));
  const map = new Map<string, string[]>();
  for (const p of meta.packages) {
    for (const t of p.targets) {
      const name = t.name.replaceAll("-", "_");
      map.set(name, [...new Set([...(map.get(name) ?? []), p.name])]);
    }
  }
  return map;
}

function command(args: string[], root: string): string {
  const run = Bun.spawnSync(args, { cwd: root });
  if (run.exitCode !== 0) throw new Error(`${args.join(" ")}: ${run.stderr.toString()}`);
  return run.stdout.toString();
}

/** Collect committed and working-tree changes without interpreting path bytes as lines. */
export function changedFiles(root: string, base: string | undefined, local: boolean) {
  const files = new Set<string>();
  const deleted = new Set<string>();
  const collect = (args: string[], output: Set<string>) => {
    for (const file of command(args, root).split("\0")) if (file) output.add(file);
  };
  if (base) {
    collect(["git", "diff", "--name-only", "-z", "--no-renames", `${base}...HEAD`], files);
    collect(["git", "diff", "--name-only", "-z", "--no-renames", "--diff-filter=D", `${base}...HEAD`], deleted);
  }
  if (local) {
    collect(["git", "diff", "--name-only", "-z", "--no-renames", "HEAD"], files);
    collect(["git", "diff", "--name-only", "-z", "--no-renames", "--diff-filter=D", "HEAD"], deleted);
    collect(["git", "ls-files", "--others", "--exclude-standard", "-z"], files);
  }
  return { files, deleted };
}

function main() {
  const root = resolve(import.meta.dir, "..");
  const baseAt = Bun.argv.indexOf("--base");
  const base = baseAt < 0 ? undefined : Bun.argv[baseAt + 1];
  const local = Bun.argv.includes("--local");
  if (!base && !local) throw new Error("supply --base <commit>, --local, or both");
  if (baseAt >= 0 && (!base || base.startsWith("--"))) throw new Error("--base requires a commit");
  const packages = rootPackages(root);
  const { files, deleted } = changedFiles(root, base, local);
  const changed = [...files].sort();
  const impact = rustImpact(changed, packages, [...deleted].sort());
  const hit = firstHit(changed, gpuiInputs(root));
  const gpui = deleted.size
    ? { run: true, reason: "deleted or renamed input requires the full GPUI matrix" }
    : hit
      ? { run: true, reason: `${hit[0]} is under ${hit[1] || "<repo root>"}` }
      : { run: false, reason: "no changed file matches the existing GPUI inputs" };
  // --failures <cargo test log>: the shadow comparison. A failed package the plan left out is
  // a miss, which is what must stay at zero before CI may skip packages.
  const logAt = Bun.argv.indexOf("--failures");
  let shadow = {};
  if (logAt > 0) {
    const failed = failedPackages(readFileSync(Bun.argv[logAt + 1], "utf8"), targetMap(root));
    const missed = failed.filter((p) => !impact.packages.includes(p));
    shadow = { failed, missed };
    if (missed.length) console.error(`ci-impact: MISS — the plan skipped failing package(s): ${missed.join(", ")}`);
  }
  console.log(JSON.stringify({ base: base ?? null, local, files: changed, rust: impact, gpui, ...shadow, note: "Rust runtime reads are modeled from path literals. tools/ci-plan.ts owns the complete verification plan." }, null, 2));
}

// --scope: the `cargo test` package arguments for $GITHUB_OUTPUT. Any error, and a missing
// base, runs the whole workspace.
function scope() {
  const root = resolve(import.meta.dir, "..");
  const baseAt = Bun.argv.indexOf("--base");
  const base = baseAt < 0 ? "" : Bun.argv[baseAt + 1] ?? "";
  if (!base || base.startsWith("--")) return { scope: "--workspace", reason: "no base commit" };
  const { files, deleted } = changedFiles(root, base, false);
  const impact = rustImpact([...files].sort(), rootPackages(root), [...deleted].sort());
  const reason = impact.mode === "all" ? impact.reasons[0] : `${impact.packages.length} of ${impact.packages.length + impact.skipped.length} packages`;
  return { scope: testScope(impact), reason };
}

if (import.meta.main) {
  if (Bun.argv.includes("--scope")) {
    let out = { scope: "--workspace", reason: "" };
    try { out = scope(); } catch (error) { out.reason = `planner failed: ${error}`; }
    console.error(`ci-impact: cargo test ${out.scope || "(no package)"}: ${out.reason}`);
    console.log(`scope=${out.scope}`);
  } else {
    try { main(); } catch (error) {
      console.error(`ci-impact: ${error}; run full verification`);
      process.exitCode = 1;
    }
  }
}
