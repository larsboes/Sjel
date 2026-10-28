// tools/ci-changes.ts — decide which path-selective CI jobs a pull request needs.
//
// Only the GPUI matrix is selective. It runs `cargo check` alone, so its inputs are the files
// rustc reads: the GPUI crates, their path dependencies, the workspace manifests those inherit
// from, and the files their sources reach outside the crate (include_str!, #[path], and the
// build script's `../../docs`). Every one of these is derived from `cargo metadata` and the
// sources, never listed, so a new path dependency is covered from the commit that adds it.
//
// The root workspace jobs stay unconditional: its tests read repository files at run time
// through CARGO_MANIFEST_DIR, which no static scan can enumerate.
//
//   tools/ci-changes --base <sha>   print `gpui=true|false` for $GITHUB_OUTPUT
//   tools/ci-changes                no base (a push): print `gpui=true`
//
// Every doubt resolves to `true`: a skipped check that was needed is the failure this guards.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const ALWAYS = [".github/workflows/ci.yml", "rust-toolchain.toml", ".cargo", "tools/ci-changes", "tools/ci-changes.ts"];

interface Package {
  dir: string;
  workspaceManifest: string;
  pathDeps: string[];
}

function cargoPackages(root: string, manifest: string): Package[] {
  const run = Bun.spawnSync(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", manifest], { cwd: root });
  if (run.exitCode !== 0) throw new Error(`cargo metadata ${manifest}: ${run.stderr.toString()}`);
  const meta = JSON.parse(run.stdout.toString());
  return meta.packages.map((p: any) => ({
    dir: dirname(p.manifest_path),
    workspaceManifest: join(meta.workspace_root, "Cargo.toml"),
    pathDeps: p.dependencies.filter((d: any) => d.path).map((d: any) => d.path),
  }));
}

function rustFiles(dir: string): string[] {
  return readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((e) => e.isFile() && e.name.endsWith(".rs") && !join(e.parentPath, e.name).includes("/target/"))
    .map((e) => join(e.parentPath, e.name));
}

/** String literals that climb out of their directory, e.g. `"../../docs"` or `"/../x"`. */
export function upwardLiterals(source: string): string[] {
  return [...source.matchAll(/"\/?((?:\.\.\/)+[^"]*)"/g)].map((m) => m[1]);
}

/** Repo-relative paths whose change can change what the GPUI matrix compiles. */
export function gpuiInputs(root: string): string[] {
  const root_ = resolve(root);
  const all = new Map<string, Package>();
  for (const manifest of ["Cargo.toml", "apps/gpui/Cargo.toml"]) {
    for (const p of cargoPackages(root_, join(root_, manifest))) all.set(p.dir, p);
  }
  const queue = cargoPackages(root_, join(root_, "apps/gpui/Cargo.toml")).map((p) => p.dir);
  const seen = new Set<string>();
  while (queue.length) {
    const dir = queue.pop()!;
    if (seen.has(dir)) continue;
    seen.add(dir);
    const p = all.get(dir);
    // A path dependency outside both workspaces is unknown, so its whole directory counts.
    if (p) queue.push(...p.pathDeps);
  }

  const inputs = new Set(ALWAYS);
  for (const dir of seen) {
    inputs.add(relative(root_, dir));
    const p = all.get(dir);
    if (p) inputs.add(relative(root_, p.workspaceManifest));
    for (const file of rustFiles(dir)) {
      for (const literal of upwardLiterals(readFileSync(file, "utf8"))) {
        // include_str! resolves against the file, a build script's join against the crate.
        for (const base of [dirname(file), dir]) {
          const target = relative(root_, resolve(base, literal));
          if (!target.startsWith("..")) inputs.add(target.replace(/\/$/, ""));
        }
      }
    }
  }
  return [...inputs].sort();
}

/** The first changed file under an input, with the input it hit; null when none does. */
export function firstHit(changed: string[], inputs: string[]): [string, string] | null {
  for (const file of changed) {
    for (const input of inputs) {
      if (input === "" || file === input || file.startsWith(`${input}/`)) return [file, input];
    }
  }
  return null;
}

/** Decide whether the GPUI matrix must run for a committed diff. */
export function decide(root: string, base: string | undefined): boolean {
  if (!base) {
    console.error("ci-changes: no base commit, so every job runs");
    return true;
  }
  const diff = Bun.spawnSync(["git", "diff", "--name-only", "-z", "--no-renames", `${base}...HEAD`], { cwd: root });
  if (diff.exitCode !== 0) {
    console.error(`ci-changes: git diff failed, so every job runs: ${diff.stderr.toString()}`);
    return true;
  }
  const removed = Bun.spawnSync(["git", "diff", "--name-only", "-z", "--no-renames", "--diff-filter=D", `${base}...HEAD`], { cwd: root });
  if (removed.exitCode !== 0 || removed.stdout.length > 0) {
    console.error("ci-changes: deleted or renamed input, or deletion detection failed, so GPUI runs");
    return true;
  }
  const changed = diff.stdout.toString().split("\0").filter(Boolean);
  const hit = firstHit(changed, gpuiInputs(root));
  console.error(hit ? `ci-changes: gpui runs: ${hit[0]} is under ${hit[1] || "<repo root>"}` : `ci-changes: gpui skipped: none of ${changed.length} changed files is a GPUI input`);
  return hit !== null;
}

if (import.meta.main) {
  const at = Bun.argv.indexOf("--base");
  const base = at > 0 ? Bun.argv[at + 1] : undefined;
  const root = join(import.meta.dir, "..");
  let gpui = true;
  try {
    gpui = decide(root, base);
  } catch (error) {
    console.error(`ci-changes: ${error}, so every job runs`);
  }
  console.log(`gpui=${gpui}`);
}
