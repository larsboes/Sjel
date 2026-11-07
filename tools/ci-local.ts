import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseJobs, type WorkflowJob, type WorkflowStep } from "./lib/ci-workflow.ts";
import { runWithReceipt } from "./lib/ci-receipts.ts";

const HELP = `tools/ci-local — replay safe CI jobs or run pre-push checks.

  tools/ci-local list [--json]     list jobs and local replay refusals
  tools/ci-local run <job>...      replay every declared shell step of each safe job
  tools/ci-local fast             impacted format checks, publication hygiene, cheap gates

  --root <dir>                    checkout to read and run in
  --base <ref>                    fast: compare against this ref (default: origin/main)
  --committed                     fast: exclude uncommitted and untracked changes
  --full                          fast: select all inputs and bypass format receipts

Tests, clippy and builds are deferred to CI, not run by fast.
Exit 0 = all passed, 1 = a check failed, 2 = usage or refused replay.`;

interface Result { step: string; ok: boolean; ms: number }
const secs = (ms: number) => `${(ms / 1000).toFixed(1)}s`;

function runStep(step: WorkflowStep, root: string): Result {
  const started = Bun.nanoseconds();
  const proc = Bun.spawnSync({
    cmd: ["bash", "--noprofile", "--norc", "-e", "-c", step.script],
    cwd: step.workingDirectory ? resolve(root, step.workingDirectory) : root,
    stdout: "inherit", stderr: "inherit", stdin: "ignore",
  });
  return { step: step.name, ok: proc.exitCode === 0, ms: Math.round((Bun.nanoseconds() - started) / 1e6) };
}

async function main(): Promise<number> {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) { console.log(HELP); return 0; }
  let root = resolve(import.meta.dir, "..");
  let base = "origin/main";
  let committed = false;
  let full = false;
  let json = false;
  const positional: string[] = [];
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--root" || arg === "--base") {
      const value = args[++i];
      if (!value || value.startsWith("-")) throw new Error(`${arg} needs a value`);
      if (arg === "--root") root = resolve(value); else base = value;
    } else if (arg === "--committed") committed = true;
    else if (arg === "--full") full = true;
    else if (arg === "--json") json = true;
    else if (arg.startsWith("-")) throw new Error(`unknown option '${arg}'`);
    else positional.push(arg);
  }
  const command = positional[0] ?? "list";
  if (command === "fast") {
    const { createPlan } = await import("./ci-plan.ts");
    const { localChecks } = await import("./ci-run.ts");
    const plan = createPlan(root, base, full, !committed);
    const checks = localChecks(plan);
    console.log(`ci-local fast: ${plan.files.length} changed file(s) against ${base}\n`);
    if (plan.full) console.log("  mode  full input selection; format receipt reuse disabled");
    for (const check of checks) console.log(`  run   ${check.name.padEnd(28)} ${check.reason}`);
    console.log("  defer tests, clippy, builds and platform checks to CI\n");
    const wallStart = Bun.nanoseconds();
    const results = await Promise.all(checks.map(async (check) => {
      const started = Bun.nanoseconds();
      let out = "", err = "";
      const result = await runWithReceipt(root, check.cmd, async () => {
        const proc = Bun.spawn({ cmd: check.cmd, cwd: root, stdout: "pipe", stderr: "pipe", stdin: "ignore" });
        const [stdout, stderr, code] = await Promise.all([
          new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited,
        ]);
        out = stdout; err = stderr;
        return code;
      }, { full: full || plan.full });
      const ms = Math.round((Bun.nanoseconds() - started) / 1e6);
      console.log(`--- ${check.name}: ${result.code === 0 ? "ok" : "FAILED"}${result.reused ? " (reused successful format check)" : ""}  ${secs(ms)}`);
      if (result.code !== 0) console.log(out + err);
      return { step: check.name, ok: result.code === 0, ms };
    }));
    const failed = results.filter((r) => !r.ok);
    const wall = secs(Math.round((Bun.nanoseconds() - wallStart) / 1e6));
    if (!failed.length) {
      console.log(`\nci-local fast: ${results.length} checks passed in ${wall}. Tests, clippy and builds remain deferred to CI.`);
      return 0;
    }
    console.error(`\nci-local fast: ${failed.length} of ${results.length} checks FAILED in ${wall}: ${failed.map((r) => r.step).join(", ")}`);
    return 1;
  }

  const workflow = resolve(root, ".github/workflows/ci.yml");
  if (!existsSync(workflow)) throw new Error(`no workflow at ${workflow}`);
  const jobs = parseJobs(readFileSync(workflow, "utf8"));
  if (command === "list") {
    if (json) console.log(JSON.stringify(jobs, null, 2));
    else {
      console.log(`${workflow}\n`);
      for (const job of jobs) {
        console.log(`  ${job.id.padEnd(14)} ${job.refusal ? "refused" : `runs here (${job.steps.length} steps)`}`);
        if (job.refusal) console.log(`  ${"".padEnd(14)} ${job.refusal}`);
        else for (const step of job.steps) console.log(`  ${"".padEnd(14)}   ${step.name}`);
      }
    }
    return 0;
  }
  if (command !== "run") throw new Error(`unknown command '${command}'\n\n${HELP}`);
  const names = positional.slice(1);
  if (!names.length) throw new Error("run needs a job name");
  const selected: WorkflowJob[] = names.map((name) => {
    const job = jobs.find((j) => j.id === name);
    if (!job) throw new Error(`ci.yml has no job '${name}'. Known: ${jobs.map((j) => j.id).join(", ")}`);
    if (job.refusal) throw new Error(`refusing to run '${name}' on this machine.\n  ${job.refusal}`);
    if (!job.steps.length) throw new Error(`job '${name}' has no run: steps — nothing would be checked.`);
    return job;
  });
  const results: Result[] = [];
  const wallStart = Bun.nanoseconds();
  for (const job of selected) {
    console.log(`\n=== ${job.id} — ${job.label} (${job.steps.length} steps)\n`);
    for (const step of job.steps) {
      console.log(`--- ${step.name}`);
      const result = runStep(step, root);
      results.push(result);
      console.log(`    ${result.ok ? "ok" : "FAILED"}  ${secs(result.ms)}\n`);
    }
  }
  const failed = results.filter((r) => !r.ok);
  const wall = secs(Math.round((Bun.nanoseconds() - wallStart) / 1e6));
  if (!failed.length) {
    console.log(`ci-local: ${results.length} steps passed in ${wall} (${names.join(", ")}).`);
    return 0;
  }
  console.error(`ci-local: ${failed.length} of ${results.length} steps FAILED in ${wall}:`);
  for (const result of failed) console.error(`  ${result.step}`);
  return 1;
}

try { process.exitCode = await main(); } catch (error) {
  console.error(`tools/ci-local: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 2;
}
