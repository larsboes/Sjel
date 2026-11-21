import { execFileSync } from "node:child_process";
import { mkdtempSync, rmdirSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join } from "node:path";
import { pinnedWithin, protectedTarget, resolvePath, type Scope } from "./classify.ts";

function fingerprint(path: string): string | undefined {
  try {
    const s = statSync(path, { bigint: true });
    return `${s.dev}:${s.ino}:${s.size}:${s.mtimeNs}:${s.ctimeNs}`;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
}

/** Detect direct writes based on missing or stale reads without recording file contents. */
export class FileObservations {
  private observed = new Map<string, string>();
  private reading = new Map<string, { path: string; stamp: string | undefined }>();

  read(id: string, path: string) {
    this.reading.set(id, { path, stamp: fingerprint(path) });
  }

  finishRead(id: string, failed: boolean) {
    const read = this.reading.get(id);
    this.reading.delete(id);
    if (!read || failed) return;
    const stamp = fingerprint(read.path);
    if (stamp && stamp === read.stamp) this.observed.set(read.path, stamp);
  }

  beforeWrite(path: string): string | undefined {
    const current = fingerprint(path);
    if (current === undefined) return undefined;
    const observed = this.observed.get(path);
    if (!observed) return "Read the existing file before editing or replacing it. No approval is needed for the read.";
    if (observed !== current) return "The file changed since its last read. Read it again and reconcile concurrent changes before writing; do not overwrite them.";
    return undefined;
  }

  finishWrite(path: string) {
    const stamp = fingerprint(path);
    if (stamp) this.observed.set(path, stamp);
    else this.observed.delete(path);
  }
}

/** Only live descendants of this Pi process qualify for automatic process control. */
export function ownedProcessIds(): number[] {
  try {
    const lines = execFileSync("/bin/ps", ["-axo", "pid=,ppid="], { encoding: "utf8", timeout: 1000, maxBuffer: 1024 * 1024 });
    const parents = new Map(lines.trim().split("\n").map((line) => {
      const [pid, parent] = line.trim().split(/\s+/).map(Number);
      return [pid, parent];
    }));
    return [...parents.keys()].filter((pid) => {
      let next = parents.get(pid);
      const visited = new Set<number>([pid]);
      while (next && !visited.has(next)) {
        if (next === process.pid) return true;
        visited.add(next);
        next = parents.get(next);
      }
      return false;
    });
  } catch {
    return [];
  }
}

/** Private scratch belongs to one extension lifetime, never to a shared build tree. */
export function createScratch(): string {
  return mkdtempSync(join(tmpdir(), "pi-task-"));
}

/** Remove empty scratch only; nonempty output remains available after reload or shutdown. */
export function releaseScratch(path: string) {
  try {
    rmdirSync(path);
  } catch (error) {
    if (!["ENOENT", "ENOTEMPTY", "EEXIST"].includes((error as NodeJS.ErrnoException).code ?? "")) throw error;
  }
}

/** Discovery limits only narrow the search; every other GIT_ variable can point Git at another repository. */
const GIT_DISCOVERY_LIMITS = new Set(["GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM"]);

function gitEnv(): NodeJS.ProcessEnv {
  return Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_") || GIT_DISCOVERY_LIMITS.has(key)));
}

/** Git failed for a reason other than the directory not being a checkout. */
export class GitUnavailable extends Error {}

/** Resolve the current worktree without allowing Git environment overrides to pick another. */
export function checkoutRoot(scope: Scope): string | undefined {
  let result: string;
  try {
    result = execFileSync("git", ["-C", scope.cwd, "rev-parse", "--show-toplevel"], {
      env: gitEnv(), encoding: "utf8", timeout: 1000, stdio: ["ignore", "pipe", "pipe"],
    }).trimEnd();
  } catch (error) {
    const stderr = String((error as { stderr?: unknown }).stderr ?? "");
    if ((error as { status?: unknown }).status === 128 && /not a git repository/i.test(stderr)) return undefined;
    throw new GitUnavailable(stderr.trim() || (error as Error).message);
  }
  if (!isAbsolute(result)) return;
  const root = resolvePath(result, scope);
  if (root !== "/" && root !== resolvePath(scope.home, scope) && pinnedWithin(scope.cwd, scope, [root])) return root;
}

/** Metadata-only Git checkpoint of `root`; failure is reported, not treated as a clean tree. */
export function dirtyFiles(scope: Scope, root: string | undefined): Map<string, string | undefined> | undefined {
  if (!root) return;
  try {
    const output = execFileSync("git", ["--no-optional-locks", "-C", root, "status", "--porcelain=v1", "-z", "--untracked-files=all"], {
      env: gitEnv(), encoding: "utf8", timeout: 2000, maxBuffer: 1024 * 1024,
    });
    const entries = output.split("\0");
    const paths: string[] = [];
    for (let i = 0; i < entries.length; i++) {
      if (!entries[i]) continue;
      paths.push(resolvePath(entries[i].slice(3), { ...scope, cwd: root }));
      if (/^[RC]|^.[RC]/.test(entries[i])) i++;
    }
    return new Map(paths.map((path) => [path, fingerprint(path)]));
  } catch {
    return undefined;
  }
}

/** Postflight warnings are advisory and cannot roll back shell or external effects. */
export function checkpointWarning(before: Map<string, string | undefined> | undefined, scope: Scope, root: string | undefined): string | undefined {
  const after = dirtyFiles(scope, root);
  if (!after || !before) return "Git checkpoint unavailable; changed-file verification did not run. Verify the task diff before claiming completion.";
  const guarded = [...after].filter(([path, stamp]) => protectedTarget(path) && (!before.has(path) || before.get(path) !== stamp));
  if (!guarded.length) return undefined;
  return `Protected agent/configuration paths changed during this run: ${JSON.stringify(guarded.map(([path]) => path))}. Verify explicit authorization and inspect their diffs. This is an advisory checkpoint, not evidence that shell effects were contained.`;
}
