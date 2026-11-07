import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const workflow = (name: string) => Bun.YAML.parse(
  readFileSync(join(import.meta.dir, "../.github/workflows", `${name}.yml`), "utf8"),
) as any;
const pages = workflow("pages");
const security = workflow("security");
const codeql = workflow("codeql");
const distribution = workflow("distribution");
const step = (job: any, name: string) => job.steps.find((entry: any) => entry.name === name);

for (const [name, definition, outputs] of [
  ["pages", pages, ["pages"]],
  ["security", security, ["deps", "images"]],
  ["codeql", codeql, ["codeql", "codeql_run"]],
] as const) {
  describe(`${name} plan contract`, () => {
    test("plans every triggered event with history and read-only Actions access", () => {
      const plan = definition.jobs.plan;
      expect(plan.if).toBeUndefined();
      expect(plan.permissions).toEqual({ contents: "read", actions: "read" });
      const checkout = plan.steps.find((entry: any) => entry.uses?.startsWith("actions/checkout@"));
      expect(checkout.with["fetch-depth"]).toBe(0);
      expect(checkout.with.ref).toBeUndefined();
      const bun = plan.steps.find((entry: any) => entry.uses?.startsWith("oven-sh/setup-bun@"));
      expect(bun.with["bun-version"]).toBe("latest");
      const planner = plan.steps.find((entry: any) => entry.id === "plan");
      expect(planner.if).toBeUndefined();
      expect(planner.env.GH_TOKEN).toBe("${{ github.token }}");
      expect(planner.run).toContain(`bun tools/ci-plan.ts --workflow ${name}.yml`);
      expect(planner.run).toContain('--output "$RUNNER_TEMP/');
      expect(planner.run).toContain('--github-output "$GITHUB_OUTPUT"');
      expect(planner.run).not.toContain("${{");
      expect(Object.keys(plan.outputs).sort()).toEqual([...outputs].sort());
      for (const output of outputs) {
        expect(plan.outputs[output]).toBe(`\${{ steps.plan.outputs.${output} }}`);
      }
      expect(definition.on.push.paths).toBeUndefined();
      expect(definition.on.push["paths-ignore"]).toBeUndefined();
      expect(definition.on.pull_request?.paths).toBeUndefined();
      expect(definition.on.pull_request?.["paths-ignore"]).toBeUndefined();
      expect(Object.hasOwn(definition.on, "workflow_dispatch")).toBe(true);
    });
  });
}

test("workflow shell scripts parse without running builds or scanners", () => {
  for (const definition of [pages, security, codeql, distribution]) {
    for (const job of Object.values(definition.jobs) as any[]) {
      for (const entry of job.steps ?? []) {
        if (!entry.run) continue;
        const result = Bun.spawnSync(["bash", "-n"], { stdin: new TextEncoder().encode(entry.run), stdout: "pipe", stderr: "pipe" });
        expect(result.stderr.toString()).toBe("");
        expect(result.exitCode).toBe(0);
      }
    }
  }
});

test("Pages gates build and deploy without permitting PR, schedule or release deployments", () => {
  expect(pages.jobs.build.needs).toBe("plan");
  expect(pages.jobs.build.if).toBe("needs.plan.outputs.pages == 'true'");
  expect(pages.jobs.deploy.needs).toEqual(["plan", "build"]);
  const publish = "github.event_name == 'push' || github.event_name == 'workflow_dispatch'";
  expect(pages.jobs.deploy.if).toBe(`needs.plan.outputs.pages == 'true' && (${publish})`);
  expect(step(pages.jobs.build, "Configure Pages").if).toBe(publish);
  expect(step(pages.jobs.build, "Upload artifact").if).toBe(publish);
  expect(pages.jobs.build["continue-on-error"]).toBeUndefined();
  expect(pages.jobs.deploy["continue-on-error"]).toBeUndefined();
  expect(pages.concurrency["cancel-in-progress"]).toBe("${{ github.event_name == 'pull_request' }}");
  expect(pages.concurrency.group).toBe("pages-${{ github.event_name == 'pull_request' && github.ref || 'main' }}");
  const build = pages.jobs.build.steps;
  expect(build.filter((entry: any) => entry.run).map((entry: any) => entry.run.trim())).toContain("tools/self check");
  expect(step(pages.jobs.build, "Assemble the site").run).toBe("website/build");
  expect(step(pages.jobs.build, "Payload hygiene").run).toBe("tools/check-site-payload.sh site");
});

test("weekly scanner refreshes reach the shared planner without path or event filters", () => {
  expect(security.on.schedule).toEqual([{ cron: "41 5 * * 1" }]);
  expect(codeql.on.schedule).toEqual([{ cron: "17 4 * * 1" }]);
  for (const definition of [security, codeql]) {
    expect(definition.jobs.plan.if).toBeUndefined();
    const planner = definition.jobs.plan.steps.find((entry: any) => entry.id === "plan");
    expect(planner.run).not.toContain("--base");
    expect(planner.run).not.toContain("git diff");
    expect(planner.run).not.toContain("--local");
  }
});

test("dependency findings remain fatal and scanners retain their scope and configuration", () => {
  for (const [job, output] of [[security.jobs.deps, "deps"], [security.jobs.images, "images"]] as const) {
    expect(job.needs).toBe("plan");
    expect(job.if).toBe(`needs.plan.outputs.${output} == 'true'`);
    expect(job["continue-on-error"]).toBeUndefined();
  }
  const scan = step(security.jobs.deps, "Scan");
  expect(scan["continue-on-error"]).toBeUndefined();
  expect(scan.run).toContain("osv-scanner scan source -r --config osv-scanner.toml");
  for (const excluded of ["target", "node_modules", "to-integrate"]) {
    expect(scan.run).toContain(`--experimental-exclude ${excluded}`);
  }
  expect(step(security.jobs.images, "Scan")["continue-on-error"]).toBe(true);
  expect(step(security.jobs.images, "The scan covered something")["continue-on-error"]).toBeUndefined();
});

test("CodeQL skips an empty include matrix at job level, not after starting each language", () => {
  const analyze = codeql.jobs.analyze;
  expect(analyze.needs).toBe("plan");
  expect(analyze.if).toBe("needs.plan.outputs.codeql_run == 'true'");
  expect(analyze.strategy.matrix).toBe("${{ fromJSON(needs.plan.outputs.codeql) }}");
  expect(analyze.strategy["fail-fast"]).toBe(false);
  expect(analyze.permissions).toEqual({ "security-events": "write", actions: "read", contents: "read" });
  expect(step(analyze, "Initialize CodeQL").with).toEqual({
    languages: "${{ matrix.language }}", "build-mode": "none", queries: "security-extended",
  });
  expect(step(analyze, "Analyze").with.category).toBe("/language:${{ matrix.language }}");
  for (const entry of analyze.steps) {
    expect(entry.if).toBeUndefined();
    expect(entry.id).not.toBe("scope");
    expect(entry["continue-on-error"]).toBeUndefined();
  }
});

test("distribution requires successful full CI at the exact caller revision before preparing artifacts", () => {
  expect(Object.keys(distribution.on)).toEqual(["workflow_dispatch"]);
  const preflight = distribution.jobs.preflight;
  expect(preflight.uses).toBe("./.github/workflows/ci.yml");
  expect(preflight.with).toEqual({ full: true });
  const ci = workflow("ci");
  expect(ci.on.workflow_call.inputs.full.type).toBe("boolean");
  expect(ci.on.workflow_call.inputs.full.default).toBe(true);
  expect(preflight.steps).toBeUndefined();
  expect(preflight["continue-on-error"]).toBeUndefined();
  expect(distribution.permissions).toEqual({ contents: "read", actions: "read" });
  expect(distribution.jobs.prepare.needs).toBe("preflight");
  expect(distribution.jobs.prepare.if).toBeUndefined();
  for (const [name, job] of Object.entries(distribution.jobs) as [string, any][]) {
    if (name !== "preflight") expect(job.needs).toBe("preflight");
  }
  const checkout = distribution.jobs.prepare.steps.find((entry: any) => entry.uses?.startsWith("actions/checkout@"));
  expect(checkout.with?.ref).toBeUndefined();
});

function imageFixture(check: (root: string, env: Record<string, string>) => void) {
  const root = mkdtempSync(join(tmpdir(), "sjel-security-workflow-"));
  try {
    check(root, { ...process.env, RUNNER_TEMP: root, GITHUB_STEP_SUMMARY: join(root, "summary.md") } as Record<string, string>);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
const runShell = (script: string, root: string, env: Record<string, string>) => Bun.spawnSync(
  ["bash", "-e", "-c", script], { cwd: root, env, stdout: "pipe", stderr: "pipe" },
);

test("image discovery fails closed for missing or empty discovery records", () => {
  imageFixture((root, env) => {
    const guard = step(security.jobs.images, "The scan covered something").run;
    expect(runShell(guard, root, env).exitCode).toBe(1);
    writeFileSync(join(root, "scanned.txt"), "");
    expect(runShell(guard, root, env).exitCode).toBe(1);
    writeFileSync(join(root, "scanned.txt"), "example.invalid/image:latest\n");
    expect(runShell(guard, root, env).exitCode).toBe(0);
  });
});

for (const [exitCode, verdict] of [[0, "clean at or above high"], [2, "findings at or above high"], [1, "COULD NOT SCAN (grype exit 1)"]] as const) {
  test(`image scanner exit ${exitCode} reports its distinct verdict without a network call`, () => {
    imageFixture((root, env) => {
      mkdirSync(join(root, "crates/example"), { recursive: true });
      mkdirSync(join(root, "bin"));
      writeFileSync(join(root, "crates/example/service.toml"), 'image = "example.invalid/image"\ntag = "stable"\n');
      writeFileSync(join(root, "bin/grype"), `#!/bin/sh\nprintf '%s\\n' '{"matches":[]}'\nexit ${exitCode}\n`, { mode: 0o755 });
      env.PATH = `${join(root, "bin")}:${env.PATH}`;
      const result = runShell(step(security.jobs.images, "Scan").run, root, env);
      expect(result.exitCode).toBe(exitCode === 0 ? 0 : 1);
      expect(readFileSync(env.GITHUB_STEP_SUMMARY, "utf8")).toContain(verdict);
      expect(readFileSync(join(root, "scanned.txt"), "utf8")).toBe("example.invalid/image:stable\n");
      expect(runShell(step(security.jobs.images, "The scan covered something").run, root, env).exitCode).toBe(0);
    });
  });
}
