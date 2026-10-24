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
  review <results.jsonl>    Render a run as Markdown for the human adjudicator
  retrieval <manifest.json> Score vector and graph retrieval on the cases' required sections

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
  const arity: Record<string, number> = { check: 0, input: 1, replay: 1, sandbox: 0, grade: 2, run: 1, review: 1, retrieval: 1 };
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
  if (name === "review") return { code: 0, output: reviewMarkdown(readBounded(rest[0]), findCase) };
  if (name === "retrieval") return retrievalCheck(JSON.parse(readBounded(rest[0])), findCase);
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
  // A broker transcript can be "completed" for an attempt that never reached the model.
  const unfinished = summary.filter((a: any) => a.termination !== "final").length;
  return { code: unfinished ? 1 : 0, output: JSON.stringify({ results: file, factual_review: "pending", unfinished_attempts: unfinished, attempts: summary }, null, 2) + "\n" };
}

/** Retrieval alone, no chat model: the case prompt is the query, and a required section counts
 * as found when a hit starts at its line. Decides whether graph expansion earns its place. */
async function retrievalCheck(raw: any, findCase: (id: string) => EvalCase): Promise<{ code: number; output: string }> {
  const { Retriever, sections } = await import("./lib/agent-eval-retrieval.ts");
  if (!raw?.retrieval?.embedding_role || !raw?.sjel?.binary || !raw?.output_dir) throw new Error("manifest needs sjel.binary, retrieval.embedding_role and output_dir");
  const rows = [];
  for (const c of (raw.cases as string[]).map(findCase)) {
    const required = (c.oracle.required_sections ?? []) as { path: string; contains: string }[];
    if (!required.length) continue;
    const files = Object.fromEntries(Object.entries(c.input.files).map(([p, f]) => [p, f.content]));
    const all = sections(files);
    const targets = required.map((r) => all.find((sec) => sec.path === r.path && sec.text.includes(r.contains)));
    if (targets.some((t) => !t)) throw new Error(`${c.id}: a required section is not in the corpus`);
    const retriever = await Retriever.build(files, raw.sjel.binary, raw.retrieval.embedding_role, join(raw.output_dir, "retrieval-cache"));
    for (const mode of ["vector", "graph"] as const) {
      const hits = await retriever.search(c.prompt, mode);
      const found = targets.filter((t) => hits.some((h) => h.path === t!.path && h.line === t!.line));
      rows.push({ case: c.id, mode, sections: retriever.size, required: targets.length, found: found.length, missing: targets.filter((t) => !found.includes(t)).map((t) => `${t!.path}:${t!.line}`), hits: hits.length, returned_chars: hits.reduce((n, h) => n + h.text.length, 0), found_via: hits.filter((h) => found.some((t) => t!.path === h.path && t!.line === h.line)).map((h) => h.via) });
    }
  }
  return { code: 0, output: JSON.stringify(rows, null, 2) + "\n" };
}

/** One section per case: the expected claims as checkboxes, then each attempt's answer and calls.
 * The adjudicator marks claims; nothing here scores an answer. */
function reviewMarkdown(jsonl: string, findCase: (id: string) => EvalCase): string {
  const attempts = jsonl.split("\n").filter(Boolean).map((line) => JSON.parse(line));
  const out = [`# Agent-eval run for review`, "", `Results: ${attempts.length} attempts, started ${attempts[0]?.started ?? "?"}. Mark each claim per attempt: supported, contradicted, unsupported or missing.`, ""];
  for (const id of [...new Set(attempts.map((a) => a.case_id as string))]) {
    const c = findCase(id);
    out.push(`## ${id}: ${c.scenario}`, "", `> ${c.prompt.replace(/\n/g, "\n> ")}`, "", "Expected claims:", "");
    for (const claim of c.oracle.claims) out.push(`- **${claim.id}**${claim.critical ? " (critical)" : ""}: ${claim.text}`);
    for (const a of attempts.filter((x) => x.case_id === id)) {
      out.push("", `### ${a.harness}, repeat ${a.repeat}: ${a.termination}, ${a.elapsed_ms} ms, ${a.model}`, "");
      for (const claim of c.oracle.claims) out.push(`- [ ] ${claim.id}: `);
      out.push("", "Tool calls:", "");
      for (const call of a.calls) out.push(`- \`${call.tool}\` → ${call.status}${call.status === "ok" ? "" : `: ${call.content.slice(0, 200)}`}`);
      if (!a.calls.length) out.push("- none");
      if (a.error) out.push("", `Error: ${a.error}`);
      out.push("", "Answer:", "", ...String(a.answer).trim().split("\n").map((line: string) => `> ${line}`));
    }
    out.push("");
  }
  return out.join("\n") + "\n";
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
