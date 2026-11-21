import { expect, test } from "bun:test";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parse } from "./ship.ts";

test("options refuse an untagged version, an unknown step and skipping publication", () => {
  expect(() => parse(["0.1.0"], "/x")).toThrow("usage");
  expect(() => parse(["v0.1.0", "--skip", "nothing"], "/x")).toThrow("cannot skip");
  expect(() => parse(["v0.1.0", "--skip", "publish"], "/x")).toThrow("cannot skip");
  expect(() => parse(["v0.1.0", "--identity", "Apple Development: x"], "/x")).toThrow("Developer ID");
  expect(parse(["v0.1.0", "--skip", "cli", "--publish"], "/x")).toMatchObject({ skip: ["cli"], publish: true });
});

test("a run resumes from saved state and publishes only with --publish", () => {
  const scratch = mkdtempSync(join(tmpdir(), "sjel-ship-"));
  try {
    const repo = join(scratch, "repo");
    mkdirSync(join(repo, "tools/distribution"), { recursive: true });
    for (const name of ["ship.ts", "notarize.ts"]) copyFileSync(join(import.meta.dir, name), join(repo, "tools/distribution", name));
    writeFileSync(join(repo, "tools/work"), '#!/bin/bash\necho "$*" >> "$FIXTURE/calls"\nmkdir -p docs/releases/receipts && echo {} > "docs/releases/receipts/$2.json"\n');
    chmodSync(join(repo, "tools/work"), 0o755);
    const git = (cwd: string, ...args: string[]) => {
      const child = Bun.spawnSync(["git", "-c", "core.hooksPath=/dev/null", "-c", "user.name=F", "-c", "user.email=f@example.invalid", ...args], { cwd, stdout: "pipe", stderr: "pipe" });
      expect(child.stderr.toString()).not.toContain("fatal");
      return child.stdout.toString().trim();
    };
    git(scratch, "init", "-q", "--bare", "origin.git");
    git(repo, "init", "-q");
    git(repo, "add", ".");
    git(repo, "commit", "-qm", "fixture");
    git(repo, "tag", "-a", "v0.0.2", "-m", "v0.0.2");
    git(repo, "remote", "add", "origin", join(scratch, "origin.git"));
    git(repo, "push", "-q", "origin", "v0.0.2");
    const bin = join(scratch, "bin"); mkdirSync(bin);
    // The draft exists; `isDraft` flips once `release edit` has run.
    writeFileSync(join(bin, "gh"), `#!/bin/bash
echo "gh $*" >> "$FIXTURE/calls"
case "$1 $2" in
  "release view") if [ -f "$FIXTURE/published" ]; then echo '{"isDraft":false,"url":"u"}'; else echo '{"isDraft":true,"url":"u"}'; fi ;;
  "release edit") touch "$FIXTURE/published" ;;
  *) exit 64 ;;
esac
`);
    chmodSync(join(bin, "gh"), 0o755);
    const home = join(scratch, "releases");
    const ship = (...args: string[]) => Bun.spawnSync([process.execPath, join(repo, "tools/distribution/ship.ts"), "v0.0.2", ...args], {
      cwd: repo, env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, FIXTURE: scratch, SJEL_RELEASE_HOME: home }, stdout: "pipe", stderr: "pipe",
    });
    const skips = ["cli", "macos", "notarize", "testflight"].flatMap((name) => ["--skip", name]);
    const first = ship(...skips);
    expect(first.stderr.toString()).not.toContain("failed");
    expect(first.exitCode).toBe(75);
    expect(first.stdout.toString()).toContain("pending  publish: review the draft at u");
    expect(existsSync(join(home, "v0.0.2/src/tools/distribution/ship.ts"))).toBe(true);
    const state = JSON.parse(readFileSync(join(home, "v0.0.2/state.json"), "utf8"));
    expect(Object.keys(state.done).sort()).toEqual(["cli", "draft", "macos", "notarize", "testflight", "worktree"]);
    const second = ship("--publish");
    expect(second.stderr.toString()).toBe("");
    expect(second.exitCode).toBe(0);
    expect(second.stdout.toString()).not.toContain("done     worktree");
    const calls = readFileSync(join(scratch, "calls"), "utf8");
    expect(calls.match(/release edit v0.0.2 --draft=false/g)).toHaveLength(1);
    expect(calls).toContain("receipt v0.0.2");
  } finally { rmSync(scratch, { recursive: true, force: true }); }
});
