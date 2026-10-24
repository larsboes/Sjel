#!/usr/bin/env bun
import { spawnSync } from "node:child_process";
import { appendFileSync, closeSync, constants, fstatSync, mkdirSync, openSync, readSync } from "node:fs";
import { join } from "node:path";
import { replayCase, solverInput, suiteSummary, validateSuite, type EvalCase } from "./lib/agent-eval.ts";

const SUITE = new URL("../research/benchmarks/agent-harness/cases.json", import.meta.url);
const HELP = `Usage: bun tools/agent-eval.ts <command>
  check                     Validate development fixtures and policies
  input <case-id>           Export only the solver-facing virtual workspace
  replay <requests.json>    Evaluate scripted tool calls; factual review stays pending
  sandbox                   Probe the local OrbStack candidate-code boundary
  grade C02 <projection.py> Evaluate a submitted artifact in that boundary
  run <manifest.json>       Run Pi and/or Sjel live on the manifest's cases and model

Only run sends model requests, and only to the models a reviewed manifest names.
Scripted replay is not evidence of a live solver's actions.
`;

function readBounded(path: string | URL): string {
  const fd = openSync(path, constants.O_RDONLY | constants.O_NONBLOCK);
  try {
    if (!fstatSync(fd).isFile()) throw new Error("evaluation input must be a regular file");
    const bytes = Buffer.alloc(4_000_001);
    let used = 0;
    while (used < bytes.length) {
      const count = readSync(fd, bytes, used, bytes.length - used, null);
      if (!count) return bytes.subarray(0, used).toString("utf8");
      used += count;
    }
    throw new Error("evaluation input exceeds 4 MB");
  } finally {
    closeSync(fd);
  }
}

/** Run only the named offline operation; reject unknown commands and extra arguments.
 * @throws When arguments, fixture data or a submitted file are invalid.
 */
export async function command(args: string[]): Promise<{ code: number; output: string }> {
  const [name, ...rest] = args;
  if (name === undefined || (name === "--help" && rest.length === 0)) return { code: 0, output: HELP };
  const arity: Record<string, number> = { check: 0, input: 1, replay: 1, sandbox: 0, grade: 2, run: 1 };
  if (!Object.hasOwn(arity, name) || rest.length !== arity[name]) throw new Error(HELP);
  if (name === "sandbox") {
    const { sandboxProbe } = await import("./lib/agent-eval-sandbox.ts");
    const result = await sandboxProbe();
    return { code: result.supported ? 0 : 2, output: JSON.stringify(result, null, 2) + "\n" };
  }
  const suite = validateSuite(JSON.parse(readBounded(SUITE)));
  const findCase = (id: string): EvalCase => {
    const item = suite.cases.find((c) => c.id === id);
    if (!item) throw new Error(`unknown case: ${id}`);
    return item;
  };
  if (name === "run") return runManifest(JSON.parse(readBounded(rest[0])), findCase);
  if (name === "check") return { code: 0, output: JSON.stringify(suiteSummary(suite), null, 2) + "\n" };
  if (name === "input") return { code: 0, output: JSON.stringify(solverInput(findCase(rest[0])), null, 2) + "\n" };
  if (name === "grade") {
    if (rest[0] !== "C02") throw new Error("isolated artifact evaluation currently supports C02 only");
    const { evaluateProjection } = await import("./lib/agent-eval-sandbox.ts");
    const result = await evaluateProjection(findCase(rest[0]), readBounded(rest[1]));
    const report = { ...result, evidence_source: "isolated_candidate_evaluation", factual_pass: null, semantic_review: "pending", import_policy_review: "pending" };
    return { code: result.status === "passed" ? 0 : result.status === "failed" ? 1 : 2, output: JSON.stringify(report, null, 2) + "\n" };
  }
  const replay = JSON.parse(readBounded(rest[0]));
  const report = replayCase(findCase(replay?.case_id), replay);
  const code = report.replay_status === "failed" ? 1 : report.replay_status === "unsupported" ? 2 : 0;
  return { code, output: JSON.stringify(report, null, 2) + "\n" };
}

/** Every case × harness × repeat, harness order alternating per repeat. One JSON line per
 * attempt goes to the manifest's output directory; stdout gets a summary without answers. */
async function runManifest(raw: unknown, findCase: (id: string) => EvalCase): Promise<{ code: number; output: string }> {
  const { liveSupported, runPi, runSjel, validateManifest } = await import("./lib/agent-eval-live.ts");
  const repoRoot = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
  const manifest = validateManifest(raw, repoRoot);
  const cases = manifest.cases.map(findCase);
  const unsupported = cases.filter((c) => !liveSupported(c)).map((c) => c.id);
  if (unsupported.length) throw new Error(`no live boundary for sandboxed Python yet: ${unsupported.join(", ")}`);
  const revision = spawnSync("git", ["-C", repoRoot, "rev-parse", "--short", "HEAD"], { encoding: "utf8" }).stdout.trim();
  const started = new Date().toISOString();
  mkdirSync(manifest.output_dir, { recursive: true });
  const file = join(manifest.output_dir, `run-${started.replace(/[:.]/g, "-")}.jsonl`);
  const summary: unknown[] = [];
  for (let repeat = 0; repeat < manifest.repeats; repeat++) {
    const order = repeat % 2 ? [...manifest.harnesses].reverse() : manifest.harnesses;
    for (const c of cases) for (const harness of order) {
      const { meta, broker, answer } = harness === "pi" ? await runPi(c, manifest) : await runSjel(c, manifest, revision);
      const report = broker.report(answer, "live_harness");
      appendFileSync(file, JSON.stringify({ started, repeat, manifest, ...meta, ...report }) + "\n");
      summary.push({ case: c.id, repeat, harness, model: meta.model, endpoint: meta.endpoint, termination: meta.termination, replay_status: report.replay_status, calls: report.calls.length, policy_failures: report.policy_failures.length, elapsed_ms: meta.elapsed_ms, behavior: report.behavior?.status ?? null });
    }
  }
  return { code: 0, output: JSON.stringify({ results: file, factual_review: "pending", attempts: summary }, null, 2) + "\n" };
}

if (import.meta.main) {
  try {
    const result = await command(process.argv.slice(2));
    process.stdout.write(result.output);
    process.exitCode = result.code;
  } catch (error) {
    process.stderr.write(`agent-eval: ${error instanceof Error ? error.message : "evaluation failed"}\n`);
    process.exitCode = 2;
  }
}
