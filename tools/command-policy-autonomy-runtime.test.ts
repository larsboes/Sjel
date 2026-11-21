import { expect, test } from "bun:test";
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { checkoutRoot, checkpointWarning, dirtyFiles, ownedProcessIds } from "../Packs/security/extensions/command-policy/runtime.ts";
import { heavyCommand } from "../Packs/security/extensions/command-policy/power.ts";

test("power detection ignores object prototype names in harmless command text", () => {
  for (const command of ["constructor", "toString", "hasOwnProperty", "rg 'x|constructor|y' source.ts"]) {
    expect(heavyCommand(command)).toBeNull();
  }
});

let processInspection = false;
try {
  execFileSync("/bin/ps", ["-axo", "pid=,ppid="], { stdio: "pipe" });
  processInspection = true;
} catch {
  processInspection = false;
}

test.skipIf(!processInspection)("owned PID discovery admits a real child but never its parent", async () => {
  const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
  const stopped = new Promise<void>((resolve, reject) => { child.once("exit", () => resolve()); child.once("error", reject); });
  try {
    expect(ownedProcessIds()).toContain(child.pid);
    expect(ownedProcessIds()).not.toContain(process.pid);
    expect(ownedProcessIds()).not.toContain(process.ppid);
  } finally {
    child.kill("SIGTERM");
    await stopped;
  }
});

test.skipIf(processInspection)("unavailable process inspection grants no automatic process control", () => {
  expect(ownedProcessIds()).toEqual([]);
});

test("postflight resolves repository-relative paths from a subdirectory and reports missing evidence", () => {
  const root = mkdtempSync(join(tmpdir(), "policy-checkpoint-"));
  const subdir = join(root, "subdir");
  mkdirSync(subdir);
  const scope = { cwd: subdir, home: homedir(), writable: [] };
  try {
    execFileSync("git", ["init", "-q", root]);
    const top = checkoutRoot(scope);
    expect(top).toBeDefined();
    const before = dirtyFiles(scope, top);
    expect(before).toBeDefined();
    expect(checkpointWarning(before, scope, top)).toBeUndefined();
    writeFileSync(join(root, "AGENTS.md"), "fixture instructions");
    expect(checkpointWarning(before, scope, top)).toContain("AGENTS.md");
    const dirty = dirtyFiles(scope, top);
    expect(checkpointWarning(dirty, scope, top)).toBeUndefined();
    writeFileSync(join(root, "AGENTS.md"), "changed fixture instructions");
    expect(checkpointWarning(dirty, scope, top)).toContain("AGENTS.md");
    expect(checkpointWarning(undefined, scope, top)).toContain("verification did not run");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

function withEnv<T>(vars: Record<string, string>, run: () => T): T {
  const previous = Object.fromEntries(Object.keys(vars).map((key) => [key, process.env[key]]));
  Object.assign(process.env, vars);
  try { return run(); } finally {
    for (const [key, value] of Object.entries(previous)) if (value === undefined) delete process.env[key]; else process.env[key] = value;
  }
}

test("checkout discovery keeps ceiling limits and ignores repository overrides", () => {
  const root = mkdtempSync(join(homedir(), ".policy-ceiling-"));
  const other = mkdtempSync(join(tmpdir(), "policy-other-"));
  const scratch = join(root, "scratch");
  mkdirSync(scratch);
  const scope = { cwd: scratch, home: homedir(), writable: [] };
  try {
    execFileSync("git", ["init", "-q", root]);
    execFileSync("git", ["init", "-q", other]);
    writeFileSync(join(other, "foreign.md"), "x");
    expect(checkoutRoot(scope)).toBeDefined();
    expect(withEnv({ GIT_CEILING_DIRECTORIES: root }, () => checkoutRoot(scope))).toBeUndefined();
    const top = checkoutRoot(scope);
    const files = withEnv({ GIT_DIR: join(other, ".git"), GIT_WORK_TREE: other }, () => dirtyFiles(scope, top));
    expect([...files!.keys()].some((path) => path.includes("foreign.md"))).toBe(false);
  } finally {
    rmSync(root, { recursive: true, force: true });
    rmSync(other, { recursive: true, force: true });
  }
});

test("a Git failure is reported apart from a directory outside any checkout", () => {
  const outside = mkdtempSync(join(tmpdir(), "policy-nogit-"));
  try {
    expect(checkoutRoot({ cwd: outside, home: homedir(), writable: [] })).toBeUndefined();
    expect(() => withEnv({ PATH: "/nonexistent" }, () => checkoutRoot({ cwd: outside, home: homedir(), writable: [] }))).toThrow();
  } finally { rmSync(outside, { recursive: true, force: true }); }
});
