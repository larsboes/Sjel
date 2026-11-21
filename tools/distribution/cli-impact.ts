import { readFileSync, realpathSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { firstHit } from "../ci-changes.ts";
import { cliBinaries, selectCliSources } from "./cli-payload.ts";

const previewInputs = [
  ".github/workflows/cli-distribution.yml", ".bun-version", ".cargo", ".gitattributes", ".gitmodules",
  "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rust-toolchain", "profiles.toml",
  "tools/ci-plan.ts", "tools/ci-impact.ts", "tools/ci-changes.ts", "tools/distribution",
];

type CargoPackage = {
  manifest_path: string;
  dependencies: { path?: string; kind: string | null }[];
  targets: { name: string; kind: string[] }[];
};

function command(root: string, args: string[]): string {
  const result = Bun.spawnSync(args, { cwd: root });
  if (result.exitCode !== 0) throw new Error(`${args[0]} ${args[1]} failed`);
  return result.stdout.toString();
}

function cliCrateDirs(root: string): string[] {
  const { packages } = JSON.parse(command(root, ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"])) as { packages: CargoPackage[] };
  const byDir = new Map(packages.map((pkg) => [dirname(pkg.manifest_path), pkg]));
  const queue = cliBinaries.map((name) => {
    const owners = packages.filter((pkg) => pkg.targets.some((target) => target.name === name && target.kind.includes("bin")));
    if (owners.length !== 1) throw new Error(`cannot resolve CLI binary ${name}`);
    return dirname(owners[0].manifest_path);
  });
  const seen = new Set<string>();
  while (queue.length) {
    const dir = queue.pop()!;
    if (seen.has(dir)) continue;
    seen.add(dir);
    const pkg = byDir.get(dir);
    if (!pkg || pkg.targets.some((target) => target.kind.some((kind) => kind === "custom-build" || kind === "proc-macro"))) {
      throw new Error(`cannot bound CLI build inputs for ${relative(root, dir)}`);
    }
    for (const dep of pkg.dependencies) if (dep.path && dep.kind !== "dev") queue.push(dep.path);
  }
  return [...seen].map((dir) => relative(root, dir));
}

function compilePaths(source: string, file: string): { path: string; rust: boolean }[] {
  const paths: { path: string; rust: boolean }[] = [];
  for (const match of source.matchAll(/\binclude(?:_str|_bytes)?\s*!\s*[(\[{]\s*|#\s*\[\s*(?:cfg_attr\s*\([^\]]*?,\s*)?path\s*=\s*/g)) {
    const arg = source.slice(match.index + match[0].length);
    if (arg.startsWith('\\"')) continue;
    const literal = arg.match(/^(?:"([^"\\\n]*)"|r(#*)"([^"\n]*)"\2)/);
    if (!literal) throw new Error(`non-literal Rust source input in ${file}`);
    paths.push({ path: literal[1] ?? literal[3], rust: !match[0].startsWith("include_") });
  }
  return paths;
}

/** Select the preview when a diff reaches its packaged files or CLI build inputs. Uncertainty runs the matrix. */
export function cliImpact(root: string, files: string[], deleted: string[]): { run: boolean; reason: string } {
  if (deleted.length) return { run: true, reason: "Deletion or rename may affect inputs absent from the current tree." };
  const direct = firstHit(files, previewInputs);
  if (direct) return { run: true, reason: `${direct[0]} affects preview configuration.` };
  if (files.length === 0) return { run: false, reason: "No changed files since the comparison base." };
  try {
    root = realpathSync(root);
    const payload = firstHit(files, selectCliSources(root).paths);
    if (payload) return { run: true, reason: `${payload[0]} is part of the CLI payload.` };
    const dirs = cliCrateDirs(root);
    const crate = firstHit(files, dirs);
    if (crate) return { run: true, reason: `${crate[0]} belongs to a CLI build dependency.` };
    const inputs = new Set<string>();
    for (const source of command(root, ["git", "ls-files", "-z", "--", "*.rs"]).split("\0").filter(Boolean)) {
      const owner = firstHit([source], dirs)?.[1];
      if (owner === undefined) continue;
      for (const input of compilePaths(readFileSync(join(root, source), "utf8"), source)) {
        for (const base of [dirname(source), owner]) {
          const path = relative(root, resolve(root, base, input.path));
          if (input.rust && !firstHit([path], dirs)) throw new Error(`external Rust source included by ${source}`);
          if (path !== ".." && !path.startsWith("../")) inputs.add(path);
        }
      }
    }
    const include = firstHit(files, [...inputs]);
    if (include) return { run: true, reason: `${include[0]} matches a compile-time input of CLI Rust sources.` };
    return { run: false, reason: `None of ${files.length} changed file(s) match CLI preview inputs.` };
  } catch (error) {
    return { run: true, reason: `CLI input discovery unavailable: ${error}` };
  }
}
