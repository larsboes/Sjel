// tools/discover-ui-packages.ts — which UI packages this repository can type-check, derived
// from checked-in package metadata instead of a list of capability names (Axon#139).
//
// The gap this closes: CI ran `bun run check` in dashboard/ and nowhere else, so a
// capability that grows its own Svelte surface is unchecked from the day it lands. The
// knowledge-graph UI demonstrated it — six TypeScript errors sitting in main while CI
// reported green, because no job knew that package existed.
//
// A hand-list of capability names would close today's instance and reopen the class the
// next time a capability grows a UI. So discovery reads what is
// already committed: a package.json is how a UI declares itself, `scripts.check` is how it
// declares it can be type-checked, and the lockfile is how it declares the check is
// reproducible. Nothing here names a capability.
//
// TypeScript rather than bash under README.md#language-tooling and the tools/doctor
// precedent: this parses package.json, and tools/lib/toml.sh's single-line grep contract is
// a TOML reader, not a JSON one.
//
//   tools/discover-ui-packages           # one row per package, with its verdict
//   tools/discover-ui-packages --dirs    # the checkable directories, one per line, for CI
//
// Exit 0 = every package is either checkable or explicitly not a UI. Exit 1 = at least one
// package declares a UI it cannot prove, or nothing checkable was found at all.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";

// Directories that can hold a package.json which is not a package of ours: installed
// dependencies, generated framework output, and build trees. Walking into node_modules
// alone would turn a three-package repository into several thousand.
const SKIP_DIRS = new Set([
  ".git",
  ".svelte-kit",
  "build",
  "dist",
  "node_modules",
  "target",
]);

// bun.lock is the current text format; bun.lockb is the binary one bun wrote before 1.2.
// Both are a committed resolution, which is all `--frozen-lockfile` needs.
const LOCKFILES = ["bun.lock", "bun.lockb"];

export type Verdict = "checkable" | "not-a-ui" | "no-check-script" | "no-lockfile" | "untracked";

export interface UiPackage {
  /** Repository-relative directory holding the package.json. */
  dir: string;
  /** package.json `name`, falling back to the directory when a package is unnamed. */
  name: string;
  verdict: Verdict;
  /** Why this verdict, in the words the failing operator needs. */
  reason: string;
}

/** A verdict that is a policy violation rather than a description. */
export function isViolation(verdict: Verdict): boolean {
  return verdict === "no-check-script" || verdict === "no-lockfile" || verdict === "untracked";
}

interface PackageJson {
  name?: string;
  scripts?: Record<string, string>;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
}

/** Every directory under `root` that owns a package.json, skipping the trees above. */
function packageDirs(root: string): string[] {
  const found: string[] = [];
  const walk = (dir: string) => {
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      // An unreadable directory is not a UI package; a permissions problem elsewhere in
      // the tree must not decide whether this gate runs.
      return;
    }
    if (entries.some((e) => e.isFile() && e.name === "package.json")) found.push(dir);
    for (const entry of entries) {
      // isDirectory() is false for a symlink, and the walk deliberately does not follow
      // one: a symlink back into the tree would rediscover every package through a second
      // path. Bazel's bazel-* root symlinks were the case that proved it, before PRD Q44
      // retired Bazel on 2026-08-25; a checkout that ever ran it still has them.
      if (entry.isDirectory() && !SKIP_DIRS.has(entry.name)) walk(join(dir, entry.name));
    }
  };
  walk(root);
  return found.sort();
}

/**
 * Is `relPath` in Git's index?
 *
 * Not "is it untracked" (`ls-files --others`), which reports nothing for a file that is
 * ignored — and an ignored lockfile is exactly the case that matters: it exists on the
 * machine that wrote it and is absent from every clone. That is the local-green/CI-red
 * shape tools/check-generator-inputs-tracked.sh names for generator inputs, and a lockfile
 * CI cannot see fails the install instead of the check, which reads as infrastructure noise
 * rather than the missing commit it is.
 *
 * Returns true when Git is unavailable: this check is about the commit, and a tree that is
 * not a checkout has none to report on.
 */
function isTracked(root: string, relPath: string): boolean {
  const git = spawnSync("git", ["-C", root, "ls-files", "--", relPath], { encoding: "utf8" });
  if (git.status !== 0) return true;
  return git.stdout.trim() !== "";
}

/** Classify one package directory. `root` is the repository root; `dir` is absolute. */
function classify(root: string, dir: string): UiPackage {
  const rel = relative(root, dir);
  const manifestPath = join(dir, "package.json");

  let pkg: PackageJson;
  try {
    pkg = JSON.parse(readFileSync(manifestPath, "utf8")) as PackageJson;
  } catch (err) {
    // Unparseable metadata is a violation, not a skip: the package claims to be one and
    // the gate cannot read what it claims.
    return {
      dir: rel,
      name: rel,
      verdict: "no-check-script",
      reason: `package.json does not parse (${err instanceof Error ? err.message : String(err)})`,
    };
  }

  const name = pkg.name ?? rel;
  const check = pkg.scripts?.check;
  const deps = { ...pkg.dependencies, ...pkg.devDependencies };
  // Either signal is enough. A UI can carry svelte only transitively through its adapter
  // while still shipping .svelte files, and the config file is the framework's own
  // declaration that this directory is a Svelte project.
  const isSvelte = "svelte" in deps || existsSync(join(dir, "svelte.config.js"));

  if (!check) {
    // A Svelte UI with no check script cannot enter this gate, and silently skipping it
    // would rebuild the false green one directory over. A package that is not a UI at all
    // is a different thing and says so.
    return isSvelte
      ? {
          dir: rel,
          name,
          verdict: "no-check-script",
          reason: "declares Svelte but no `check` script, so nothing can type-check it",
        }
      : {
          dir: rel,
          name,
          verdict: "not-a-ui",
          reason: "no `check` script and no Svelte — nothing declared to check",
        };
  }

  const lockfile = LOCKFILES.find((f) => existsSync(join(dir, f)));
  if (!lockfile) {
    return {
      dir: rel,
      name,
      verdict: "no-lockfile",
      reason: "declares a `check` script but commits no bun lockfile, so the install it needs is not reproducible",
    };
  }

  for (const file of ["package.json", lockfile]) {
    if (!isTracked(root, join(rel, file))) {
      return {
        dir: rel,
        name,
        verdict: "untracked",
        reason: `${file} is not in the index — this package exists on this machine and in no clone (local green, CI red)`,
      };
    }
  }

  return { dir: rel, name, verdict: "checkable", reason: `\`bun run check\` → ${check}` };
}

/** Every package under `root`, classified. Sorted by directory, so output is stable. */
export function discoverUiPackages(root: string): UiPackage[] {
  return packageDirs(root).map((dir) => classify(root, dir));
}

function main(argv: string[]): number {
  const dirsOnly = argv.includes("--dirs");
  // SJEL_UI_DISCOVERY_ROOT is the test seam, mirroring SJEL_PUBLICATION_ROOT in
  // check-publication-hygiene.sh: the walk is the behavior under test, so the test plants
  // a scratch tree and points the tool at it rather than at the real checkout.
  const root = resolve(
    process.env.SJEL_UI_DISCOVERY_ROOT ?? join(import.meta.dir, ".."),
  );

  const packages = discoverUiPackages(root);
  const checkable = packages.filter((p) => p.verdict === "checkable");
  const violations = packages.filter((p) => isViolation(p.verdict));

  if (dirsOnly) {
    // The consumer is a shell loop, so a refused package must not produce a usable list:
    // print nothing and exit non-zero rather than a set that silently omits it and lets
    // the loop report success over reduced coverage.
    if (violations.length === 0) for (const p of checkable) console.log(p.dir);
  } else {
    for (const p of packages) {
      const mark = p.verdict === "checkable" ? "ok  " : p.verdict === "not-a-ui" ? "skip" : "FAIL";
      console.log(`${mark} ${p.dir} — ${p.reason}`);
    }
  }

  if (checkable.length === 0) {
    console.error(
      "discover-ui-packages: no checkable UI package found — the dashboard alone should produce one, so discovery is broken rather than the repository empty",
    );
    return 1;
  }
  if (violations.length > 0) {
    console.error(
      `discover-ui-packages: ${violations.length} package(s) declare a UI this gate cannot check: ${violations
        .map((p) => p.dir)
        .join(", ")}`,
    );
    return 1;
  }
  if (!dirsOnly) {
    console.log(
      `discover-ui-packages: ${checkable.length} checkable package(s), ${packages.length - checkable.length} skipped.`,
    );
  }
  return 0;
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
