import { expect, test } from "bun:test";
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { checkpointWarning, dirtyFiles, ownedProcessIds } from "../Packs/security/extensions/command-policy/runtime.ts";
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
    const before = dirtyFiles(scope);
    expect(before).toBeDefined();
    expect(checkpointWarning(before, scope)).toBeUndefined();
    writeFileSync(join(root, "AGENTS.md"), "fixture instructions");
    expect(checkpointWarning(before, scope)).toContain("AGENTS.md");
    const dirty = dirtyFiles(scope);
    expect(checkpointWarning(dirty, scope)).toBeUndefined();
    writeFileSync(join(root, "AGENTS.md"), "changed fixture instructions");
    expect(checkpointWarning(dirty, scope)).toContain("AGENTS.md");
    expect(checkpointWarning(undefined, scope)).toContain("verification did not run");
  } finally { rmSync(root, { recursive: true, force: true }); }
});
