import { expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { changedFiles, failedPackages, fastPlan, namedPaths, rustImpact, type Package } from "./ci-impact.ts";
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

test("widens workspace-wide inputs and a deleted unowned Rust source", () => {
  for (const file of ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"]) {
    expect(rustImpact([file], packages).mode).toBe("all");
  }
  expect(rustImpact([], packages, ["libs/gone/src/lib.rs"]).mode).toBe("all");
});

test("a build script, manifest or fixture selects its owner", () => {
  for (const file of ["libs/base/build.rs", "libs/base/Cargo.toml", "libs/base/fixture.json"]) {
    expect(rustImpact([file], packages).packages).toEqual(["app", "base", "top"]);
  }
});

test("does not confuse sibling path prefixes", () => {
  expect(rustImpact(["libs/base-extra/src/lib.rs"], packages).packages).toEqual([]);
});

test("a file outside every package selects the packages that name it, and only those", () => {
  const reading: Package[] = [
    { ...packages[0], reads: ["docs/plans"] },
    { ...packages[2], reads: ["."] },
    packages[1], packages[3],
  ];
  expect(rustImpact(["docs/plans/x.md"], reading).packages).toEqual(["app", "base", "other", "top"]);
  expect(rustImpact(["docs/plansx.md"], reading).packages).toEqual(["other"]);
  expect(rustImpact(["README.md"], packages).packages).toEqual([]);
});

test("namedPaths resolves upward literals and keeps top-level names", () => {
  const top = new Set(["docs", "Packs", "README.md"]);
  const src = `let a = "../../docs/plans"; let b = "Packs"; let c = "application/json"; let d = "../..";`;
  expect(namedPaths(src, "libs/base/src/lib.rs", "libs/base", top).sort()).toEqual([".", "Packs", "docs/plans", "libs", "libs/docs/plans"]);
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
  expect(workflow.jobs.verification.if).toBe("always()");
  expect(workflow.jobs.verification.needs).toEqual(["changes", "rust-quality", "cargo-tests", "bun-tests", "ui-check", "repo-gates", "gpui-targets"]);
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

test("fast plan: docs-only change skips clippy, Rust change scopes it, deletion widens it", () => {
  const none = () => false;
  const names = (p: ReturnType<typeof fastPlan>) => p.checks.map((c) => c.name);
  expect(names(fastPlan(["docs/x.md"], [], packages, none))).toEqual(["cargo fmt", "repo gates"]);
  expect(names(fastPlan(["libs/base/src/lib.rs"], [], packages, none))).toContain("cargo test");
  const scoped = fastPlan(["libs/base/src/lib.rs"], [], packages, none).checks.find((c) => c.name === "clippy")!;
  expect(scoped.cmd).toContain("-p");
  expect(scoped.cmd).not.toContain("--workspace");
  expect(fastPlan(["Cargo.lock"], [], packages, none).checks.find((c) => c.name === "clippy")!.cmd).toContain("--workspace");
  expect(fastPlan(["libs/gone/src/lib.rs"], ["libs/gone/src/lib.rs"], packages, none).checks.find((c) => c.name === "clippy")!.cmd).toContain("--workspace");
  expect(names(fastPlan(["apps/gpui/src/main.rs"], [], packages, none))).not.toContain("clippy");
});

test("fast plan: picks tests beside changed files", () => {
  const exists = (p: string) => ["tools/x.test.ts", "tools/x.test.sh"].includes(p);
  const plan = fastPlan(["tools/x.ts", "tools/x"], [], packages, exists);
  expect(plan.checks.find((c) => c.name === "bun test")!.cmd).toEqual(["bun", "test", "./tools/x.test.ts"]);
  expect(plan.checks.map((c) => c.name)).toContain("tools/x.test.sh");
});

test("failedPackages names a failing test binary's package and a crate that did not compile", () => {
  const targets = new Map([["sjel_cli", ["sjel-cli"]], ["civil_date", ["civil-date"]]]);
  const log = [
    "     Running unittests src/lib.rs (target/debug/deps/civil_date-73bf82e7ddc5d063)",
    "test result: ok. 3 passed; 0 failed",
    "     Running unittests src/main.rs (target/debug/deps/sjel_cli-a90016d6dc09e508)",
    "test result: FAILED. 1 passed; 1 failed",
    "error: could not compile `sjel-comms` (lib test) due to 1 previous error",
  ].join("\n");
  expect(failedPackages(log, targets)).toEqual(["sjel-cli", "sjel-comms"]);
});
