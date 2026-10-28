// tools/ci-local.ts — run a CI job's steps on this machine, from CI's own definition.
//
// CI has been red on this repository for stretches measured in days, and the only way to
// learn that was to push and wait. The repo-gates job takes about twenty seconds here;
// the push-and-wait cycle around it is minutes. That gap is the whole reason this exists.
//
// THE STEP LIST IS NOT WRITTEN DOWN HERE. It is read out of .github/workflows/ci.yml,
// because a second copy of a gate list is a copy that goes stale — the same argument
// ci.yml already makes for discovering `tools/*.test.sh` with a glob rather than listing
// them (Sjel#116). Add a gate to the repo-gates job and it runs here on the next
// invocation, with nobody editing this file.
//
// What is safe to replay locally is a judgement, not a derivation, and it lives in
// tools/lib/ci-workflow.ts beside the reason for each verdict. This file owns the I/O and
// the process handling and nothing else.
//
//   tools/ci-local list             every job, and whether it runs here
//   tools/ci-local run repo-gates   CI's repo gates, in CI's order
//   tools/ci-local run bun-tests    bun test, then every tools/*.test.sh
//
// Exit 0 = every step passed. 1 = a step failed. 2 = usage, or a refused job.

import { existsSync, readFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { parseJobs, type WorkflowJob, type WorkflowStep } from "./lib/ci-workflow.ts";
import { changedFiles, fastPlan, rootPackages } from "./ci-impact.ts";

const HELP = `tools/ci-local — run a CI job's steps on this machine, read from ci.yml.

  tools/ci-local list [--json]     every job in ci.yml, and whether it runs here
  tools/ci-local run <job>...      run each named job's \`run:\` steps, in order
  tools/ci-local fast              the quick subset for this change, in parallel: fmt, repo
                                   gates, clippy and cargo test on affected crates, tests beside
                                   changed files

  --root <dir>                     the checkout to read and run in (default: this one)
  --base <ref>                     fast: compare against this ref (default: origin/main)
  --committed                      fast: ignore uncommitted and untracked files

Every step runs even after one fails, so one red gate does not hide the rest.
Exit 0 = all passed, 1 = a step failed, 2 = usage or a refused job.`;

interface Result {
  step: string;
  ok: boolean;
  ms: number;
}

/**
 * Run one step the way a runner does: bash, `-e`, output straight through.
 *
 * GitHub's default shell for a Linux `run:` block is `bash -e {0}`, and a step that sets
 * more (`set -uo pipefail`, which the shell-tests loop does) still gets exactly what it
 * asks for.
 *
 * Output is inherited rather than captured, so a suite that takes minutes prints as it
 * goes. The `::group::` lines a step emits are GitHub's fold markers and appear here as
 * plain text — left alone, because rewriting a step's output is the first move toward
 * running something other than what CI runs.
 */
function runStep(step: WorkflowStep, cwd: string): Result {
  const started = Bun.nanoseconds();
  // A step that declares working-directory: runs there in CI, so it must run there here,
  // or the command differs between the two. GitHub resolves it against the checkout root,
  // which is what `cwd` is. join() is the right operator rather than resolve(): an absolute
  // working-directory is legal in a workflow and resolve() would honour it, but joining also
  // keeps a leading "./" and any ".." honest without silently escaping the checkout.
  const stepCwd = step.workingDirectory ? join(cwd, step.workingDirectory) : cwd;
  const proc = Bun.spawnSync({
    cmd: ["bash", "--noprofile", "--norc", "-e", "-c", step.script],
    cwd: stepCwd,
    stdout: "inherit",
    stderr: "inherit",
    stdin: "ignore",
  });
  return { step: step.name, ok: proc.exitCode === 0, ms: Math.round((Bun.nanoseconds() - started) / 1e6) };
}

const secs = (ms: number) => `${(ms / 1000).toFixed(1)}s`;

// --- CLI -------------------------------------------------------------------

const argv = process.argv.slice(2);
if (argv.includes("-h") || argv.includes("--help")) {
  console.log(HELP);
  process.exit(0);
}

// --root exists so tools/ci-local.test.sh can plant a workflow whose gates fail and watch
// this refuse it, without a failing gate anywhere in the real checkout.
let root = resolve(import.meta.dir, "..");
const rootAt = argv.indexOf("--root");
if (rootAt !== -1) {
  const given = argv[rootAt + 1];
  if (!given) {
    console.error("tools/ci-local: --root needs a directory");
    process.exit(2);
  }
  root = resolve(given);
  argv.splice(rootAt, 2);
}

const wantJson = argv.includes("--json");
const positional = argv.filter((a) => !a.startsWith("-"));
const cmd = positional[0] ?? "list";

const workflow = `${root}/.github/workflows/ci.yml`;
if (!existsSync(workflow)) {
  console.error(`tools/ci-local: no workflow at ${workflow}`);
  process.exit(2);
}

let jobs: WorkflowJob[];
try {
  jobs = parseJobs(readFileSync(workflow, "utf8"));
} catch (error) {
  console.error(`tools/ci-local: ${workflow}: ${error instanceof Error ? error.message : error}`);
  process.exit(2);
}

if (cmd === "list") {
  if (wantJson) {
    console.log(JSON.stringify(jobs, null, 2));
    process.exit(0);
  }
  console.log(`${workflow}\n`);
  for (const job of jobs) {
    const plural = job.steps.length === 1 ? "" : "s";
    console.log(`  ${job.id.padEnd(14)} ${job.refusal ? "refused" : `runs here (${job.steps.length} step${plural})`}`);
    if (job.refusal) console.log(`  ${"".padEnd(14)} ${job.refusal}`);
    else for (const step of job.steps) console.log(`  ${"".padEnd(14)}   ${step.name}`);
  }
  process.exit(0);
}

if (cmd === "fast") {
  const baseAt = argv.indexOf("--base");
  const base = baseAt === -1 ? "origin/main" : argv[baseAt + 1];
  if (!base || base.startsWith("-")) {
    console.error("tools/ci-local: --base needs a commit");
    process.exit(2);
  }
  const { files, deleted } = changedFiles(root, base, !argv.includes("--committed"));
  const plan = fastPlan([...files].sort(), [...deleted].sort(), rootPackages(root), (p) => existsSync(join(root, p)));
  console.log(`ci-local fast: ${files.size} changed file(s) against ${base}\n`);
  for (const c of plan.checks) console.log(`  run   ${c.name.padEnd(28)} ${c.reason}`);
  for (const s of plan.skipped) console.log(`  skip  ${s}`);
  console.log("");

  // In parallel, with output held until a check ends, so two logs never interleave.
  const wallStart = Bun.nanoseconds();
  const outcomes = await Promise.all(plan.checks.map(async (c) => {
    const started = Bun.nanoseconds();
    const proc = Bun.spawn({ cmd: c.cmd, cwd: root, stdout: "pipe", stderr: "pipe", stdin: "ignore" });
    const [out, err, code] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited]);
    const ms = Math.round((Bun.nanoseconds() - started) / 1e6);
    console.log(`--- ${c.name}: ${code === 0 ? "ok" : "FAILED"}  ${secs(ms)}`);
    if (code !== 0) console.log(out + err);
    return { step: c.name, ok: code === 0, ms };
  }));
  const failed = outcomes.filter((r) => !r.ok);
  const wall = secs(Math.round((Bun.nanoseconds() - wallStart) / 1e6));
  if (failed.length === 0) {
    console.log(`\nci-local fast: ${outcomes.length} checks passed in ${wall}. CI still runs the full suite.`);
    process.exit(0);
  }
  console.error(`\nci-local fast: ${failed.length} of ${outcomes.length} checks FAILED in ${wall}: ${failed.map((f) => f.step).join(", ")}`);
  process.exit(1);
}

if (cmd !== "run") {
  console.error(`tools/ci-local: unknown command '${cmd}'\n\n${HELP}`);
  process.exit(2);
}

const names = positional.slice(1);
if (names.length === 0) {
  console.error(`tools/ci-local: run needs a job name\n\n${HELP}`);
  process.exit(2);
}

const selected: WorkflowJob[] = [];
for (const name of names) {
  const job = jobs.find((j) => j.id === name);
  if (!job) {
    console.error(`tools/ci-local: ci.yml has no job '${name}'. Known: ${jobs.map((j) => j.id).join(", ")}`);
    process.exit(2);
  }
  if (job.refusal) {
    console.error(`tools/ci-local: refusing to run '${name}' on this machine.`);
    console.error(`  ${job.refusal}`);
    process.exit(2);
  }
  // A job with no `run:` steps is a job this would report green over. That is the
  // empty-sweep failure tools/check-store-transactions.sh already guards against on its
  // own input: no work done reads as no problem found.
  if (job.steps.length === 0) {
    console.error(`tools/ci-local: job '${name}' has no run: steps — nothing would be checked.`);
    process.exit(2);
  }
  selected.push(job);
}

const results: Result[] = [];
const wallStart = Bun.nanoseconds();
for (const job of selected) {
  console.log(`\n=== ${job.id} — ${job.label} (${job.steps.length} steps)\n`);
  for (const step of job.steps) {
    console.log(`--- ${step.name}`);
    // Every step runs even after one fails. Stopping at the first would hide how much of
    // the repository is actually broken behind whichever gate happens to sort first —
    // ci.yml makes the same argument for its own ui-check loop.
    const result = runStep(step, root);
    results.push(result);
    console.log(`    ${result.ok ? "ok" : "FAILED"}  ${secs(result.ms)}\n`);
  }
}

const failed = results.filter((r) => !r.ok);
const wall = secs(Math.round((Bun.nanoseconds() - wallStart) / 1e6));
if (failed.length === 0) {
  console.log(`ci-local: ${results.length} steps passed in ${wall} (${selected.map((j) => j.id).join(", ")}).`);
  process.exit(0);
}
console.error(`ci-local: ${failed.length} of ${results.length} steps FAILED in ${wall}:`);
for (const f of failed) console.error(`  ${f.step}`);
process.exit(1);
