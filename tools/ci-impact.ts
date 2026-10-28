import { dirname, relative, resolve } from "node:path";
import { firstHit, gpuiInputs } from "./ci-changes.ts";

export interface Package {
  name: string;
  dir: string;
  dependencies: string[];
}

export interface Impact {
  mode: "all" | "candidate";
  packages: string[];
  reasons: string[];
  skipped: string[];
}

/** Propose Rust packages for shadow comparison; runtime file reads are not modeled. */
export function rustImpact(files: string[], packages: Package[]): Impact {
  const all = packages.map((p) => p.name).sort();
  const selected = new Set<string>();
  const reasons: string[] = [];
  for (const file of files) {
    const owner = packages.filter((p) => file.startsWith(`${p.dir}/`)).sort((a, b) => b.dir.length - a.dir.length)[0];
    if (!owner || !file.endsWith(".rs") || file.endsWith("/build.rs")) {
      return { mode: "all", packages: all, skipped: [], reasons: [`${file}: input is not a package Rust source`] };
    }
    selected.add(owner.name);
    reasons.push(`${file}: owned by ${owner.name}`);
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

export interface FastCheck {
  name: string;
  cmd: string[];
  reason: string;
}

/** Inputs the root workspace's clippy reads; apps/gpui is its own workspace and CI matrix. */
function rustInput(file: string): boolean {
  if (file.startsWith("apps/gpui/")) return false;
  const base = file.split("/").pop()!;
  return file.endsWith(".rs") || base === "Cargo.toml" || base === "Cargo.lock" ||
    file === "rust-toolchain.toml" || file === "clippy.toml" || file.startsWith(".cargo/");
}

/**
 * The quick local subset of CI for a change. Full CI still runs on every push; this only
 * catches the cheap failures before it. Every doubt about Rust widens clippy to the workspace.
 */
export function fastPlan(files: string[], deleted: boolean, packages: Package[], exists: (path: string) => boolean) {
  const checks: FastCheck[] = [
    { name: "cargo fmt", cmd: ["cargo", "fmt", "--all", "--check"], reason: "always: one second" },
    { name: "repo gates", cmd: ["tools/ci-local", "run", "repo-gates"], reason: "always: file-based, about a minute" },
  ];
  const skipped: string[] = [];

  const rust = files.filter(rustInput);
  if (rust.length === 0 && !deleted) {
    skipped.push("clippy: no root-workspace Rust input changed");
  } else {
    const impact = deleted
      ? { mode: "all" as const, packages: [], reasons: ["a deleted or renamed input"] }
      : rustImpact(rust, packages);
    const scope = impact.mode === "all" ? ["--workspace"] : impact.packages.flatMap((p) => ["-p", p]);
    checks.push({
      name: "clippy",
      cmd: ["tools/cargo-hermetic", "clippy", ...scope, "--all-targets", "--all-features", "--locked", "--", "-D", "warnings"],
      reason: impact.mode === "all" ? `workspace: ${impact.reasons[0]}` : impact.packages.join(", "),
    });
  }

  // Tests named after a changed file: x.test.ts beside x.ts, tools/x.test.sh beside tools/x.
  const bun = new Set<string>();
  const shell = new Set<string>();
  for (const file of files) {
    const stem = file.replace(/\.test\.(ts|sh)$/, "").replace(/\.(ts|sh)$/, "");
    if (exists(`${stem}.test.ts`)) bun.add(`./${stem}.test.ts`);
    if (exists(`${stem}.test.sh`)) shell.add(`${stem}.test.sh`);
  }
  if (bun.size) checks.push({ name: "bun test", cmd: ["bun", "test", ...[...bun].sort()], reason: `${bun.size} test file(s) beside changed files` });
  for (const t of [...shell].sort()) checks.push({ name: t, cmd: ["bash", t], reason: "beside a changed file" });
  if (!bun.size && !shell.size) skipped.push("bun and shell tests: none beside a changed file");

  if (files.some((f) => f.startsWith("apps/gpui/"))) skipped.push("gpui: CI's platform matrix checks it");
  skipped.push("cargo test and ui-check: CI runs them in full");
  return { checks, skipped };
}

/** The root workspace's packages and their in-workspace dependencies. */
export function rootPackages(root: string): Package[] {
  const metadata = JSON.parse(command(["cargo", "metadata", "--no-deps", "--format-version", "1"], root));
  const members = new Set<string>(metadata.workspace_members);
  const workspace = metadata.packages.filter((p: any) => members.has(p.id));
  const namesByDir = new Map<string, string>(workspace.map((p: any) => [dirname(p.manifest_path), p.name]));
  return workspace.map((p: any) => ({
    name: p.name,
    dir: relative(root, dirname(p.manifest_path)),
    dependencies: p.dependencies.flatMap((d: any) => d.path && namesByDir.has(d.path) ? [namesByDir.get(d.path)] : []),
  }));
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
  const impact = deleted.size
    ? { mode: "all", packages: packages.map((p: any) => p.name).sort(), skipped: [], reasons: [`${[...deleted].sort()[0]}: deleted or renamed input`] }
    : rustImpact(changed, packages);
  const hit = firstHit(changed, gpuiInputs(root));
  const gpui = deleted.size
    ? { run: true, reason: "deleted or renamed input requires the full GPUI matrix" }
    : hit
      ? { run: true, reason: `${hit[0]} is under ${hit[1] || "<repo root>"}` }
      : { run: false, reason: "no changed file matches the existing GPUI inputs" };
  console.log(JSON.stringify({ shadow: true, base: base ?? null, local, files: changed, rust: impact, gpui, note: "Rust is a candidate only; full CI remains required. Runtime file reads are not modeled." }, null, 2));
}

if (import.meta.main) {
  try { main(); } catch (error) {
    console.error(`ci-impact: ${error}; full checks remain required`);
    process.exitCode = 1;
  }
}
