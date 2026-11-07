import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { localChecks, runGates } from "./ci-run.ts";
import { outputs, type CIPlan } from "./ci-plan.ts";
import { allGateIDs, GATES } from "./lib/ci-inputs.ts";

const empty = (): CIPlan => ({
  version: 1, head: "a".repeat(40), base: "b".repeat(40), full: false, files: [], reasons: [],
  rust: { mode: "candidate", packages: [], skipped: [], reasons: [] }, gpui: { include: [] },
  bun: [], shell: [], ui: [], gates: [], pages: false, security: { deps: false, images: false, codeql: [] },
});

test("local checks never duplicate CI tests, clippy or build work", () => {
  const plan = empty();
  plan.rust.packages = ["example"];
  plan.gates = Object.keys(GATES);
  const checks = localChecks(plan);
  expect(checks.some((check) => check.name === "publication hygiene")).toBe(true);
  expect(checks.some((check) => check.cmd[0] === "cargo" && check.cmd[1] === "fmt")).toBe(true);
  expect(checks.some((check) => check.cmd.some((part) => ["test", "clippy", "build"].includes(part)))).toBe(false);
  const selected = checks.find((check) => check.name === "repo gates")!.cmd.at(-1)!.split(",");
  expect(selected.every((id) => GATES[id].cheap)).toBe(true);
  expect(selected).not.toContain("publication-hygiene");
});

test("empty impact skips work explicitly, never the publication safeguard", () => {
  const plan = empty();
  expect(localChecks(plan).map((check) => check.name)).toEqual(["publication hygiene"]);
  const out = outputs(plan);
  for (const key of ["rust", "quality", "bun", "ui", "gpui", "codeql_run"]) expect(out[key]).toBe("false");
  expect(JSON.parse(out.gpui_matrix)).toEqual({ include: [] });
});

test("gate runner retains later coverage after failure and rejects unknown IDs", () => {
  const root = mkdtempSync(join(tmpdir(), "sjel-ci-run-"));
  try {
    mkdirSync(join(root, "tools"));
    writeFileSync(join(root, "tools/check-service-tomls.sh"), "#!/bin/sh\nexit 1\n", { mode: 0o755 });
    writeFileSync(join(root, "tools/check-route-schemas.sh"), "#!/bin/sh\nprintf covered > covered\n", { mode: 0o755 });
    expect(runGates(root, ["service-tomls", "route-schemas"])).toBe(false);
    expect(readFileSync(join(root, "covered"), "utf8")).toBe("covered");
    expect(() => runGates(root, ["missing"])).toThrow("unknown repository gate");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("full gate discovery is independent of incomplete unrelated UI edits", () => {
  const root = mkdtempSync(join(tmpdir(), "sjel-ci-gates-"));
  try {
    expect(Bun.spawnSync(["git", "init", "-q"], { cwd: root }).exitCode).toBe(0);
    mkdirSync(join(root, "ui"));
    writeFileSync(join(root, "ui/package.json"), '{"scripts":{"check":"typecheck"}}');
    expect(allGateIDs(root)).toEqual(Object.keys(GATES).sort());
    mkdirSync(join(root, "tools"));
    writeFileSync(join(root, "tools/check-new-invariant.sh"), "#!/bin/sh\nexit 0\n");
    expect(() => allGateIDs(root)).toThrow("Unregistered repository gate");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("release full-verification requirement accepts only a successful marker at the exact commit", () => {
  const root = mkdtempSync(join(tmpdir(), "sjel-ci-full-"));
  try {
    const bin = join(root, "bin"); mkdirSync(bin);
    writeFileSync(join(bin, "gh"), `#!/bin/sh
case "$2" in
  *'/jobs?'*) printf '%s' "$JOBS" ;;
  *) printf '%s' "$RUNS" ;;
esac
`, { mode: 0o755 });
    const sha = "a".repeat(40);
    const run = (runs: unknown[], jobs: unknown[]) => Bun.spawnSync(["bun", join(import.meta.dir, "ci-plan.ts"), "--root", root, "--require-full", sha], {
      env: { PATH: `${bin}:${process.env.PATH}`, HOME: root, GITHUB_REPOSITORY: "example/fixture", RUNS: JSON.stringify({ workflow_runs: runs }), JOBS: JSON.stringify({ jobs }) },
      stdout: "pipe", stderr: "pipe",
    });
    const good = { id: 1, head_sha: sha, conclusion: "success", html_url: "https://example.invalid/runs/1" };
    const marker = { name: "full verification result", conclusion: "success" };
    expect(run([good], [marker]).exitCode).toBe(0);
    expect(run([good], [{ name: "verification result", conclusion: "success" }]).exitCode).toBe(1);
    expect(run([good], [{ ...marker, conclusion: "skipped" }]).exitCode).toBe(1);
    expect(run([{ ...good, head_sha: "b".repeat(40) }], [marker]).exitCode).toBe(1);
    expect(run([{ ...good, conclusion: "failure" }], [marker]).exitCode).toBe(1);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
