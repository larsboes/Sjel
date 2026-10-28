// tools/ci-impact-backtest.ts — test the Rust impact planner against CI's own history.
//
// For each completed CI run, take the change it tested, plan it with tools/ci-impact.ts as
// that commit stood, and read which packages actually failed `cargo test`. A failed package
// the plan left out is a miss: selective CI would have skipped a real failure. Enforcement
// waits on zero misses.
//
//   bun run tools/ci-impact-backtest.ts [--limit N] [--json]
//
// Needs `gh` with read access to the repository's Actions runs and logs.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { changedFiles, failedPackages, rootPackages, rustImpact, targetMap } from "./ci-impact.ts";

const ROOT = resolve(import.meta.dir, "..");
const JOB = "cargo test (hermetic)";

function sh(args: string[], cwd = ROOT): string {
  const run = Bun.spawnSync(args, { cwd, stderr: "pipe" });
  if (run.exitCode !== 0) throw new Error(`${args.join(" ")}: ${run.stderr.toString().trim()}`);
  return run.stdout.toString();
}

interface Row {
  run: number;
  event: string;
  head: string;
  conclusion: string;
  files: number;
  mode: string;
  selected: number;
  total: number;
  failed: string[];
  missed: string[];
  /** Full SHAs, so a row can find the run of its own base commit. */
  headSha: string;
  baseSha: string;
}

function backtest(run: any, previousPush: string | undefined): Row | null {
  const jobs = JSON.parse(sh(["gh", "run", "view", String(run.databaseId), "--json", "jobs"])).jobs;
  const job = jobs.find((j: any) => j.name === JOB);
  if (!job || !["success", "failure"].includes(job.conclusion)) return null;
  const api = JSON.parse(sh(["gh", "api", `repos/{owner}/{repo}/actions/runs/${run.databaseId}`]));
  const head = run.headSha;
  // A push can carry several commits, so its range starts at the previous push's head.
  let base = api.pull_requests?.[0]?.base?.sha ?? previousPush ?? "";
  sh(["git", "fetch", "-q", "origin", head]);
  if (!base) base = `${head}^`;
  else sh(["git", "fetch", "-q", "origin", base]);
  const baseSha = sh(["git", "rev-parse", base]).trim();

  const dir = mkdtempSync(join(tmpdir(), "sjel-backtest-"));
  try {
    sh(["git", "worktree", "add", "-q", "--detach", dir, head]);
    const { files, deleted } = changedFiles(dir, base, false);
    const packages = rootPackages(dir);
    const impact = rustImpact([...files].sort(), packages, [...deleted].sort());
    const failed = job.conclusion === "failure"
      ? failedPackages(sh(["gh", "run", "view", "--job", String(job.databaseId), "--log"]), targetMap(dir))
      : [];
    const selected = new Set(impact.packages);
    return {
      run: run.databaseId, event: run.event, head: head.slice(0, 8), conclusion: job.conclusion,
      files: files.size, mode: impact.mode, selected: impact.packages.length, total: packages.length,
      failed, missed: failed.filter((p) => !selected.has(p)), headSha: head, baseSha,
    };
  } finally {
    sh(["git", "worktree", "remove", "--force", dir]);
    rmSync(dir, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  const at = Bun.argv.indexOf("--limit");
  const limit = at > 0 ? Number(Bun.argv[at + 1]) : 100;
  const runs = JSON.parse(sh(["gh", "run", "list", "--workflow", "ci.yml", "--status", "completed", "--limit", String(limit), "--json", "databaseId,headSha,event,conclusion"]));
  const rows: Row[] = [];
  const errors: string[] = [];
  // A cancelled run tested nothing, so the range reaches back to the last push that finished.
  const pushes = runs.filter((r: any) => r.event === "push" && ["success", "failure"].includes(r.conclusion)).sort((a: any, b: any) => b.databaseId - a.databaseId);
  for (const run of runs) {
    try {
      const previous = run.event === "push" ? pushes.find((p: any) => p.databaseId < run.databaseId && p.headSha !== run.headSha)?.headSha : undefined;
      const row = backtest(run, previous);
      if (row) rows.push(row);
    } catch (error) {
      errors.push(`${run.databaseId}: ${error}`);
    }
  }
  // A package that also failed in the run of the base commit was already broken there: the
  // change did not cause it, so skipping it is not a miss.
  // When the base has no run of its own (pushes land in batches), the latest earlier main run
  // stands in for it. Run ids grow with time.
  const byHead = new Map(rows.map((r) => [r.headSha, r]));
  const mainRuns = rows.filter((r) => r.event === "push").sort((a, b) => b.run - a.run);
  for (const r of rows) {
    const base = byHead.get(r.baseSha) ?? mainRuns.find((m) => m.run < r.run);
    const preexisting = r.missed.filter((p) => base?.failed.includes(p));
    if (preexisting.length) {
      r.missed = r.missed.filter((p) => !preexisting.includes(p));
      (r as any).preexisting = preexisting;
    }
  }
  const red = rows.filter((r) => r.conclusion === "failure");
  const misses = rows.filter((r) => r.missed.length);
  const saved = rows.filter((r) => r.mode === "candidate").map((r) => 1 - r.selected / r.total);
  const summary = {
    runs: rows.length, red: red.length, runsWithMisses: misses.length,
    meanShareSkipped: saved.length ? Number((saved.reduce((a, b) => a + b, 0) / rows.length).toFixed(3)) : 0,
    selectedNone: rows.filter((r) => r.mode === "candidate" && r.selected === 0).length,
    selectedAll: rows.filter((r) => r.mode === "all" || r.selected === r.total).length,
    errors,
  };
  if (Bun.argv.includes("--json")) console.log(JSON.stringify({ summary, rows }, null, 2));
  else {
    for (const r of rows) {
      console.log(`${r.run} ${r.event.padEnd(12)} ${r.head} ${r.conclusion.padEnd(7)} files=${String(r.files).padStart(3)} ${r.mode.padEnd(9)} ${r.selected}/${r.total}` +
        (r.failed.length ? `  failed=${r.failed.join(",")}` : "") + (r.missed.length ? `  MISSED=${r.missed.join(",")}` : "") +
        ((r as any).preexisting ? `  preexisting=${(r as any).preexisting.join(",")}` : ""));
    }
    console.log(JSON.stringify(summary, null, 2));
  }
  process.exitCode = misses.length || errors.length ? 1 : 0;
}
