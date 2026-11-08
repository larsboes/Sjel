import { expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createPlan, outputs, platformChecks, successfulBase, summary, type CIPlan, type WorkflowRun } from "./ci-plan.ts";
import { rustImpact } from "./ci-impact.ts";

const members = ["sjel-gpui", "sjel-gpui-core", "sjel-gpui-ui", "sjel-gpui-ios"];
const workflowRun = (id: number, sha: string, conclusion = "success", event = "push"): WorkflowRun => ({ id, head_sha: sha, conclusion, event, html_url: `https://example.invalid/runs/${id}` });

test("push baseline excludes failed/cancelled runs, non-ancestors and PR merge commits", () => {
  const runs = [workflowRun(6, "foreign"), workflowRun(5, "pr", "success", "pull_request"), workflowRun(4, "cancelled", "cancelled"), workflowRun(3, "failed", "failure"), workflowRun(2, "good")];
  expect(successfulBase(runs, (sha) => sha !== "foreign")).toBe("good");
  expect(successfulBase(runs, () => false)).toBeUndefined();
});

test("GPUI platform plan scopes iOS-only and desktop-only changes", () => {
  const ios = platformChecks(["sjel-gpui-ios"], members);
  expect(ios.map((p) => p.name)).toEqual(["macos + ios"]);
  expect(ios[0].os).toBe("xcode-27");
  expect(ios[0].check).not.toContain("--all-targets");
  const desktop = platformChecks(["sjel-gpui"], members);
  expect(desktop.map((p) => p.name)).toEqual(["macos + ios", "linux", "windows shell"]);
  expect(desktop[0].target).toBe("");
  expect(platformChecks(members, members)).toHaveLength(5);
  expect(platformChecks([], members)).toEqual([]);
  expect(() => platformChecks(["; unsafe"], [...members, "; unsafe"])).toThrow();
});

test("a fixture owned by one crate still selects other runtime readers and their dependents", () => {
  const packages = [
    { name: "owner", dir: "crates/owner", dependencies: [] },
    { name: "reader", dir: "crates/reader", dependencies: [], reads: ["crates/owner/fixture.json"] },
    { name: "consumer", dir: "crates/consumer", dependencies: ["reader"] },
  ];
  expect(rustImpact(["crates/owner/fixture.json"], packages).packages).toEqual(["consumer", "owner", "reader"]);
});

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "sjel-ci-plan-"));
  const put = (path: string, text: string) => { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), text); };
  const git = (...args: string[]) => {
    const run = Bun.spawnSync(["git", ...args], { cwd: root });
    if (run.exitCode !== 0) throw new Error(run.stderr.toString());
    return run.stdout.toString().trim();
  };
  git("init", "-q");
  git("config", "user.name", "Fixture");
  git("config", "user.email", "fixture@example.invalid");
  put("Cargo.toml", '[workspace]\nresolver = "2"\nmembers = ["crates/base"]\n');
  put("crates/base/Cargo.toml", '[package]\nname = "base"\nversion = "0.0.0"\nedition = "2021"\n');
  put("crates/base/src/lib.rs", "pub fn value() {}\n");
  put("apps/gpui/Cargo.toml", '[workspace]\nresolver = "2"\nmembers = ["core", "ios"]\n');
  put("apps/gpui/core/Cargo.toml", '[package]\nname = "sjel-gpui-core"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\nbase = { path = "../../../crates/base" }\n');
  put("apps/gpui/core/src/lib.rs", "pub fn value() {}\n");
  put("apps/gpui/ios/Cargo.toml", '[package]\nname = "sjel-gpui-ios"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\nsjel-gpui-core = { path = "../core" }\n');
  put("apps/gpui/ios/src/lib.rs", "pub fn value() {}\n");
  put("docs/guide.md", "Guide\n");
  git("add", ".");
  git("commit", "-qm", "fixture");
  return { root, put, git, base: git("rev-parse", "HEAD") };
}

test("shared plan falls back on missing/invalid base and scopes GPUI reverse path dependencies", () => {
  const f = fixture();
  try {
    expect(createPlan(f.root, undefined).full).toBe(true);
    expect(createPlan(f.root, "not-a-ref").full).toBe(true);
    f.put("crates/base/src/lib.rs", "pub fn changed() {}\n");
    f.git("add", "."); f.git("commit", "-qm", "change base");
    const plan = createPlan(f.root, f.base);
    expect(plan.full).toBe(false);
    expect(plan.rust.packages).toEqual(["base"]);
    expect(plan.gpui.include.map((p) => p.name)).toEqual(["macos + ios", "linux", "windows", "android"]);
    const out = outputs(plan);
    expect(out.rust).toBe("true");
    expect(JSON.parse(out.gpui_matrix)).toEqual(plan.gpui);
    expect(summary(plan)).toContain("Selection reasons");
    expect(summary(plan)).toContain("affected");
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test("plan summaries retain a failed full attempt alongside the last successful full receipt", () => {
  const f = fixture();
  try {
    f.put("bin/gh", `#!/bin/sh
case "$2" in
  *'/actions/artifacts?'*) printf '%s' '{"artifacts":[{"expired":false,"workflow_run":{"id":2,"head_branch":"main"}}]}' ;;
  *'/actions/runs/2') printf '%s' '{"id":2,"head_sha":"${f.base}","event":"push","conclusion":"success","html_url":"https://example.invalid/full/2"}' ;;
  *) printf '%s' '{"workflow_runs":[{"id":3,"head_sha":"${f.base}","event":"schedule","conclusion":"failure","html_url":"https://example.invalid/full/3"}]}' ;;
esac
`);
    const chmod = Bun.spawnSync(["chmod", "+x", join(f.root, "bin/gh")]);
    expect(chmod.exitCode).toBe(0);
    const report = join(f.root, "summary.md");
    const run = Bun.spawnSync(["bun", join(import.meta.dir, "ci-plan.ts"), "--root", f.root, "--base", f.base], {
      env: { ...process.env, PATH: `${join(f.root, "bin")}:${process.env.PATH}`, GITHUB_EVENT_NAME: "push", GITHUB_REPOSITORY: "example/fixture", GITHUB_STEP_SUMMARY: report },
      stdout: "pipe", stderr: "pipe",
    });
    expect(run.stderr.toString()).toBe("");
    expect(run.exitCode).toBe(0);
    const content = readFileSync(report, "utf8");
    expect(content).toContain("Latest full-run attempt: **failure**");
    expect(content).toContain("Last successful full verification: https://example.invalid/full/2");
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test("CI triggers preserve an uncancelled daily full run and explicit reusable full mode", async () => {
  const workflow = Bun.YAML.parse(await Bun.file(join(import.meta.dir, "../.github/workflows/ci.yml")).text()) as any;
  expect(workflow.on.schedule).toEqual([{ cron: "23 3 * * *" }]);
  expect(workflow.on.release.types).toEqual(["published"]);
  expect(workflow.on.workflow_dispatch.inputs.full.default).toBe(true);
  expect(workflow.on.workflow_call.inputs.full.default).toBe(true);
  expect(workflow.concurrency.group).toContain("github.event_name");
  expect(workflow.concurrency["cancel-in-progress"]).toBe("${{ github.event_name == 'push' || github.event_name == 'pull_request' }}");
  expect(workflow.jobs["full-verification"].needs).toEqual(["changes", "verification"]);
  expect(workflow.jobs["full-verification"].if).toBe("needs.changes.outputs.full == 'true'");
  for (const job of Object.values(workflow.jobs) as any[]) {
    for (const step of job.steps ?? []) {
      if (!step.run || step.run.includes("${{")) continue;
      const checked = Bun.spawnSync(["bash", "-n"], { stdin: new TextEncoder().encode(step.run) });
      expect(checked.stderr.toString()).toBe("");
      expect(checked.exitCode).toBe(0);
    }
  }
});

test("planner edits widen every surface and local mode includes untracked files", () => {
  const f = fixture();
  try {
    f.put("tools/ci-plan.ts", "changed planner\n");
    const plan = createPlan(f.root, f.base, false, true);
    expect(plan.full).toBe(true);
    expect(plan.files).toContain("tools/ci-plan.ts");
    expect(plan.security.codeql).toHaveLength(4);
    expect(plan.pages).toBe(true);
    expect(plan.security.deps).toBe(true);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});
