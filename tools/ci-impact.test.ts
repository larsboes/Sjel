import { expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { changedFiles, rustImpact, type Package } from "./ci-impact.ts";
import { decide } from "./ci-changes.ts";

const packages: Package[] = [
  { name: "base", dir: "libs/base", dependencies: [] },
  { name: "app", dir: "apps/app", dependencies: ["base"] },
  { name: "other", dir: "libs/other", dependencies: [] },
  { name: "top", dir: "apps/top", dependencies: ["app"] },
];

test("includes transitive reverse dependencies", () => {
  expect(rustImpact(["libs/base/src/lib.rs"], packages).packages).toEqual(["app", "base", "top"]);
});

test("widens unknown and build inputs", () => {
  for (const file of ["Cargo.lock", "libs/base/build.rs", "libs/base/fixture.json"]) {
    expect(rustImpact([file], packages).mode).toBe("all");
  }
});

test("does not confuse sibling path prefixes", () => {
  expect(rustImpact(["libs/base-extra/src/lib.rs"], packages).mode).toBe("all");
});

test("reports unaffected packages explicitly", () => {
  expect(rustImpact(["libs/base/src/lib.rs"], packages).skipped).toEqual(["other"]);
});

test("collects staged, unstaged, untracked and deleted files", () => {
  const root = mkdtempSync(join(tmpdir(), "sjel-impact-"));
  const git = (...args: string[]) => {
    const result = Bun.spawnSync(["git", ...args], { cwd: root });
    if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  };
  try {
    git("init", "-q");
    git("config", "user.name", "Test");
    git("config", "user.email", "test@example.invalid");
    mkdirSync(join(root, "src"));
    for (const name of ["staged.rs", "unstaged.rs", "deleted.rs"]) writeFileSync(join(root, "src", name), "before");
    git("add", ".");
    git("commit", "-qm", "fixture");
    writeFileSync(join(root, "src", "staged.rs"), "after");
    git("add", "src/staged.rs");
    writeFileSync(join(root, "src", "unstaged.rs"), "after");
    writeFileSync(join(root, "src", "new\nfile.rs"), "new");
    unlinkSync(join(root, "src", "deleted.rs"));
    const changes = changedFiles(root, undefined, true);
    expect([...changes.files].sort()).toEqual(["src/deleted.rs", "src/new\nfile.rs", "src/staged.rs", "src/unstaged.rs"]);
    expect([...changes.deleted]).toEqual(["src/deleted.rs"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("collects committed renames as a deletion and an addition", () => {
  const root = mkdtempSync(join(tmpdir(), "sjel-impact-"));
  const git = (...args: string[]) => {
    const result = Bun.spawnSync(["git", ...args], { cwd: root });
    if (result.exitCode !== 0) throw new Error(result.stderr.toString());
    return result.stdout.toString().trim();
  };
  try {
    git("init", "-q");
    git("config", "user.name", "Test");
    git("config", "user.email", "test@example.invalid");
    writeFileSync(join(root, "old.rs"), "source");
    git("add", ".");
    git("commit", "-qm", "base");
    const base = git("rev-parse", "HEAD");
    git("mv", "old.rs", "new.rs");
    git("commit", "-qm", "rename");
    const changes = changedFiles(root, base, false);
    expect([...changes.files].sort()).toEqual(["new.rs", "old.rs"]);
    expect([...changes.deleted]).toEqual(["old.rs"]);
    expect(decide(root, base)).toBe(true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("aggregate fails for a failed, missing or unexpectedly skipped job", async () => {
  const workflow = Bun.YAML.parse(await Bun.file(join(import.meta.dir, "../.github/workflows/ci.yml")).text()) as any;
  const script = workflow.jobs.verification.steps[0].run;
  const good = {
    CHANGES: "success", RUST_QUALITY: "success", CARGO_TESTS: "success", BUN_TESTS: "success",
    UI_CHECK: "success", REPO_GATES: "success", GPUI_SELECTED: "true", GPUI: "success",
  };
  const run = (overrides: Record<string, string>) => Bun.spawnSync(["bash", "-e", "-c", script], {
    env: { ...process.env, ...good, ...overrides },
    stdout: "pipe", stderr: "pipe",
  }).exitCode;
  expect(run({})).toBe(0);
  expect(run({ GPUI_SELECTED: "false", GPUI: "skipped" })).toBe(0);
  expect(run({ BUN_TESTS: "failure" })).toBe(1);
  expect(run({ CHANGES: "skipped" })).toBe(1);
  expect(run({ GPUI_SELECTED: "true", GPUI: "skipped" })).toBe(1);
  expect(run({ GPUI_SELECTED: "", GPUI: "skipped" })).toBe(1);
});
