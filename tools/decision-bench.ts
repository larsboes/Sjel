#!/usr/bin/env bun
// Runs research/benchmarks/decisions/cases.jsonl against one local System One server and writes
// one result file per run. CLM's reference server and Ollama 0.35+ take the same request
// (Packs/harness/extensions/clm-classifier.ts, header), so one runner measures both.
//
//   bun tools/decision-bench.ts --backend ollama --model nimble [--url http://127.0.0.1:11434/v1/]
//   bun tools/decision-bench.ts --backend clm --model clm-latest [--url http://127.0.0.1:8700/v1/]
//
// One request per case, one question per request, so a case's latency is its own. The first
// case is sent once before timing starts and its answer discarded: it pays the model load.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { SJEL_ROOT } from "./lib/demo-endpoints.ts";

const SUITE_DIR = join(SJEL_ROOT, "research/benchmarks/decisions");

export interface Case {
  id: string;
  task: string;
  state: string;
  question: { type: "choice" | "noul"; instructions: string; criteria: Record<string, string> };
  expected: string | boolean;
}

export interface CaseResult {
  id: string;
  expected: string | boolean;
  /** null when the server failed or answered in a shape this runner cannot read. */
  got: string | boolean | null;
  /** choice: the server's `confidence`. noul: the probability of the answer given. */
  confidence: number | null;
  ms: number;
  error?: string;
}

/** Nearest-rank percentile. Empty input gives 0 rather than NaN, so a result file stays valid JSON. */
export function percentile(values: number[], p: number): number {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1))];
}

/** Reads one answer. A noul is true at 0.5 or above. */
export function readAnswer(c: Case, answer: unknown): Pick<CaseResult, "got" | "confidence"> {
  if (!answer || typeof answer !== "object") return { got: null, confidence: null };
  const a = answer as Record<string, unknown>;
  if (c.question.type === "noul") {
    if (typeof a.noul !== "number") return { got: null, confidence: null };
    const yes = a.noul >= 0.5;
    return { got: yes, confidence: yes ? a.noul : 1 - a.noul };
  }
  if (typeof a.choice !== "string") return { got: null, confidence: null };
  return { got: a.choice, confidence: typeof a.confidence === "number" ? a.confidence : null };
}

export function summarize(cases: CaseResult[]) {
  const correct = cases.filter((c) => c.got === c.expected).length;
  const ms = cases.filter((c) => c.got !== null).map((c) => c.ms);
  return {
    n: cases.length,
    correct,
    accuracy: cases.length ? Number((correct / cases.length).toFixed(4)) : 0,
    latency_ms: { p50: Math.round(percentile(ms, 50)), p95: Math.round(percentile(ms, 95)) },
  };
}

export function loadCases(file = join(SUITE_DIR, "cases.jsonl")): Case[] {
  return readFileSync(file, "utf8").split("\n").filter((l) => l.trim()).map((l) => JSON.parse(l) as Case);
}

async function ask(url: string, model: string, c: Case): Promise<{ answer: unknown; ms: number }> {
  const started = performance.now();
  const response = await fetch(new URL("systemone", url), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ model, state: c.state, questions: { q: c.question } }),
    signal: AbortSignal.timeout(120_000),
  });
  const ms = performance.now() - started;
  if (!response.ok) throw new Error(`HTTP ${response.status}: ${(await response.text()).slice(0, 200)}`);
  const body = (await response.json()) as { answers?: Record<string, unknown> };
  return { answer: body.answers?.q, ms };
}

/** Chip and OS version only: no host name and no user name reach a public result file. */
function hostDescription(): string {
  const read = (cmd: string, args: string[]) => {
    try {
      return execFileSync(cmd, args, { encoding: "utf8" }).trim();
    } catch {
      return "";
    }
  };
  const chip = read("sysctl", ["-n", "machdep.cpu.brand_string"]) || "unknown CPU";
  const os = read("sw_vers", ["-productVersion"]);
  return os ? `${chip}, macOS ${os}` : chip;
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const flag = (name: string) => {
    const i = args.indexOf(`--${name}`);
    return i >= 0 ? args[i + 1] : undefined;
  };
  const backend = flag("backend");
  if (backend !== "ollama" && backend !== "clm") throw new Error("--backend must be ollama or clm");
  const model = flag("model") ?? (backend === "ollama" ? "nimble" : "clm-latest");
  const url = flag("url") ?? (backend === "ollama" ? "http://127.0.0.1:11434/v1/" : "http://127.0.0.1:8700/v1/");
  const cases = loadCases();

  await ask(url, model, cases[0]); // warm-up, not timed and not scored
  const results: CaseResult[] = [];
  for (const c of cases) {
    try {
      const { answer, ms } = await ask(url, model, c);
      results.push({ id: c.id, expected: c.expected, ...readAnswer(c, answer), ms: Math.round(ms) });
    } catch (e) {
      results.push({ id: c.id, expected: c.expected, got: null, confidence: null, ms: 0,
        error: e instanceof Error ? e.message : String(e) });
    }
  }
  const date = new Date().toISOString().slice(0, 10);
  const out = {
    suite: "decisions", date, backend, model, host: hostDescription(),
    ...summarize(results), cases: results,
  };
  const file = join(SUITE_DIR, "results", `${date}-${backend}-${model.replace(/[^a-zA-Z0-9.-]+/g, "-")}.json`);
  writeFileSync(file, `${JSON.stringify(out, null, 2)}\n`);
  console.log(`${model}: ${out.correct}/${out.n} (${(out.accuracy * 100).toFixed(1)}%), p50 ${out.latency_ms.p50} ms, p95 ${out.latency_ms.p95} ms -> ${file}`);
}

if (import.meta.main) await main();
