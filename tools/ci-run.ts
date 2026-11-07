import { existsSync, readFileSync } from "node:fs";
import { availableParallelism } from "node:os";
import { join, resolve } from "node:path";
import { createPlan, type CIPlan } from "./ci-plan.ts";
import { testScope, type FastCheck } from "./ci-impact.ts";
import { allGateIDs, GATES } from "./lib/ci-inputs.ts";

/** Cheap local checks share CI's input plan; compilation and tests remain in CI. */
export function localChecks(plan: CIPlan): FastCheck[] {
  const checks: FastCheck[] = [];
  if (plan.rust.packages.length) checks.push({ name: "cargo fmt", cmd: ["cargo", "fmt", "--all", "--check"], reason: "affected root Rust packages" });
  if (plan.gpui.include.length) checks.push({ name: "gpui fmt", cmd: ["cargo", "fmt", "--manifest-path", "apps/gpui/Cargo.toml", "--all", "--check"], reason: "affected GPUI packages" });
  checks.push({ name: "publication hygiene", cmd: ["tools/check-publication-hygiene.sh"], reason: "always; never reused" });
  const ids = plan.gates.filter((id) => GATES[id]?.cheap && !GATES[id].cmd.includes("tools/check-publication-hygiene.sh"));
  if (ids.length) checks.push({ name: "repo gates", cmd: ["bun", "tools/ci-run.ts", "gates", "--ids", ids.join(",")], reason: ids.join(", ") });
  return checks;
}

function run(root: string, cmd: string[], cwd = root): boolean {
  console.log(`::group::${cmd.join(" ")}`);
  const result = Bun.spawnSync(cmd, { cwd, stdin: "ignore", stdout: "inherit", stderr: "inherit" });
  console.log("::endgroup::");
  return result.exitCode === 0;
}

/** Run every selected gate, retaining all failures instead of stopping at the first one.
 * @throws If an ID is not declared by the gate registry.
 */
export function runGates(root: string, ids: string[]): boolean {
  if (ids.some((id) => !GATES[id])) throw new Error("plan names an unknown repository gate");
  let ok = true;
  for (const id of ids) if (!run(root, GATES[id].cmd)) ok = false;
  return ok;
}

async function shellTests(root: string, tests: string[]): Promise<boolean> {
  const results: boolean[] = [];
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(availableParallelism(), tests.length) }, async () => {
    while (next < tests.length) {
      const test = tests[next++];
      const proc = Bun.spawn(["bash", test], { cwd: root, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
      const [out, err, code] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited]);
      console.log(`::group::${test}: ${code === 0 ? "passed" : "FAILED"}\n${out}${err}\n::endgroup::`);
      results.push(code === 0);
    }
  }));
  return results.every(Boolean);
}

function readPlan(root: string, full: boolean): CIPlan {
  const file = process.env.RUNNER_TEMP && join(process.env.RUNNER_TEMP, "sjel-ci-plan.json");
  if (!full && process.env.CI === "true") {
    if (!file || !existsSync(file)) throw new Error("CI plan artifact is missing");
    const plan = JSON.parse(readFileSync(file, "utf8")) as CIPlan;
    const head = Bun.spawnSync(["git", "rev-parse", "HEAD"], { cwd: root });
    if (plan.version !== 1 || head.exitCode !== 0 || plan.head !== head.stdout.toString().trim()) throw new Error("CI plan does not match this checkout");
    return plan;
  }
  return createPlan(root, undefined, true);
}

async function main() {
  const args = Bun.argv.slice(2);
  const root = resolve(import.meta.dir, "..");
  const job = args[0];
  const idsAt = args.indexOf("--ids");
  if (job === "gates" && idsAt > 0) {
    const ids = args[idsAt + 1]?.split(",").filter(Boolean);
    if (!ids?.length) throw new Error("--ids requires gate IDs");
    process.exitCode = runGates(root, ids) ? 0 : 1;
    return;
  }
  if (job === "gates" && args.includes("--full")) {
    process.exitCode = runGates(root, allGateIDs(root)) ? 0 : 1;
    return;
  }
  const plan = readPlan(root, args.includes("--full"));
  let ok = true;
  if (job === "gates") {
    ok = runGates(root, plan.gates);
  } else if (job === "rust-quality") {
    if (plan.rust.packages.length) {
      ok = run(root, ["cargo", "fmt", "--all", "--check"]);
      if (!run(root, ["bash", "-euo", "pipefail", "-c", `cargo clippy ${testScope(plan.rust)} --all-targets --all-features --locked -- -D warnings`])) ok = false;
    }
    if (plan.gpui.include.length && !run(root, ["cargo", "fmt", "--manifest-path", "apps/gpui/Cargo.toml", "--all", "--check"])) ok = false;
  } else if (job === "cargo-tests") {
    if (plan.rust.packages.length) ok = run(root, ["bash", "-euo", "pipefail", "-c", `cargo test ${testScope(plan.rust)} --locked --no-fail-fast`]);
  } else if (job === "bun-tests") {
    if (plan.bun.length) {
      for (const dir of ["Packs/harness/pi-packages/pi-subagents", "Packs/harness/pi-packages/pi-web-access"]) {
        if (existsSync(join(root, dir, "package.json")) && !run(root, ["bun", "install", "--frozen-lockfile", "--ignore-scripts"], join(root, dir))) ok = false;
      }
      if (ok && !run(root, ["bun", "test", ...plan.bun.map((file) => `./${file}`)])) ok = false;
    }
    if (!await shellTests(root, plan.shell)) ok = false;
  } else if (job === "ui-check") {
    for (const dir of plan.ui) {
      const cwd = join(root, dir);
      let installed = run(root, ["bun", "install", "--frozen-lockfile", "--ignore-scripts"], cwd);
      if (!installed) { ok = false; continue; }
      if (!run(root, ["bun", "run", "check"], cwd)) ok = false;
      if (!run(root, ["bun", "pm", "scan"], cwd)) ok = false;
      const pkg = JSON.parse(readFileSync(join(cwd, "package.json"), "utf8"));
      if (pkg.scripts?.build && !run(root, ["bun", "run", "build"], cwd)) ok = false;
      else if (!pkg.scripts?.build) console.log(`${dir}: no build script declared`);
    }
  } else {
    throw new Error("expected gates, rust-quality, cargo-tests, bun-tests, or ui-check");
  }
  console.log(`${job}: ${ok ? "passed" : "FAILED"} (${plan.full ? "full" : "affected"} plan)`);
  process.exitCode = ok ? 0 : 1;
}

if (import.meta.main) main().catch((error) => { console.error(`ci-run: ${error}`); process.exitCode = 1; });
