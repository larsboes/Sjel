#!/usr/bin/env bun
// tools/skill-eval — run a skill's evals, and check the suites are sound.
//
// The problem this exists for: Axon had 36 skills and 2 eval suites, and nothing read
// either of them. Every gate in tools/ measures STRUCTURE — is the metadata discoverable, is
// the skill self-contained, do two skills' contracts agree — and none of them can answer the
// only question that matters about a skill: does it change what the model does? A skill is an
// instruction, and an instruction that changes nothing is pure cost, because its description
// is loaded every session.
//
// The method is not invented here. It is `skill-creator`'s references/evaluation.md, which
// distilled it from agentskills.io's evaluating-skills guide and obra/superpowers' baseline
// idea. This tool is the missing executor for that method:
//
//   tools/skill-eval list                 every suite, and whether a baseline is recorded
//   tools/skill-eval check [<skill>|--all]  validate the SUITES (this half runs in CI)
//   tools/skill-eval run <skill> [flags]  execute the A/B and write a benchmark
//
// ── Why `check` is separate from `run`, and why only one of them is a gate ───────────
//
// `run` costs money, needs a model, and is nondeterministic. CI cannot have it: a gate that
// flakes teaches people to ignore gates. So the two halves are split by what can be proven
// cheaply. `check` is pure file validation and is wired into repo-gates; `run` is opt-in and
// manual, exactly like the jobs tools/ci-local refuses to replay.
//
// ── The measurement, stated honestly ────────────────────────────────────────────────
//
// Both arms are fresh `claude -p` processes in throwaway project directories, differing in
// ONE way: the treatment directory has `.claude/skills/<name>/` and the baseline does not.
// Note what this does NOT do — it does not isolate the machine's other installed skills.
// Claude Code reads `~/.claude/skills/` regardless of cwd, and the two ways to prevent that
// both fail: CLAUDE_CONFIG_DIR breaks auth (no credentials in a fresh config dir), and
// `skillOverrides: off` only unlists a skill, it does not remove it, so the model reads the
// file anyway and answers from it (verified 2026-09-17 with a probe skill).
//
// So the delta this reports is "this skill ADDED TO THE OWNER'S REAL INSTALL", not "this
// skill in a vacuum". That is the realistic question, and it matches the baseline slide-deck
// recorded by hand. It has one consequence to hold on to: a case whose task some OTHER
// installed skill already covers will show no delta, and that is a true finding about this
// install, not a broken eval.
//
// Cost, measured 2026-09-17 on this machine: ~$0.35 and ~2.5s per trivial run (55k cached
// input tokens, because the global skills and CLAUDE.md load every time). A graded case is
// four model calls — two arms, two grades — so budget roughly $1.50 per case. Run one case
// while iterating, not a whole suite.

import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { tmpdir } from "node:os";

const SJEL_ROOT = resolve(import.meta.dir, "..");
const CANONICAL_TOP_KEYS = ["skill_name", "evals"] as const;
const KNOWN_TOP_KEYS = [
  "skill_name",
  "evals",
  "note",
  "baseline",
  "results",
  "trigger_tests",
] as const;

export type EvalCase = {
  id: string | number;
  prompt: string;
  expected_output?: string;
  files?: string[];
  assertions: string[];
  edge?: boolean;
};

export type Suite = {
  skill_name?: string;
  evals?: EvalCase[];
  note?: string;
  baseline?: unknown;
  results?: unknown;
  trigger_tests?: { should_trigger?: string[]; should_not_trigger?: string[] };
  [key: string]: unknown;
};

export type Skill = { name: string; pack: string; dir: string; suitePath: string | null };

// ── discovering skills and suites ───────────────────────────────────────────────────

export function allSkills(): Skill[] {
  const out: Skill[] = [];
  for (const pack of readdirSync(join(SJEL_ROOT, "Packs"))) {
    const skillsDir = join(SJEL_ROOT, "Packs", pack, "skills");
    if (!existsSync(skillsDir)) continue;
    for (const name of readdirSync(skillsDir)) {
      const dir = join(skillsDir, name);
      if (!existsSync(join(dir, "SKILL.md"))) continue;
      const suitePath = join(dir, "evals", "evals.json");
      out.push({ name, pack, dir, suitePath: existsSync(suitePath) ? suitePath : null });
    }
  }
  return out.sort((a, b) => a.name.localeCompare(b.name));
}

function findSkill(name: string): Skill {
  const found = allSkills().find((s) => s.name === name);
  if (!found) throw new Error(`no skill named '${name}' under Packs/*/skills/`);
  return found;
}

function readSuite(skill: Skill): Suite {
  if (!skill.suitePath) {
    throw new Error(`${skill.name} has no evals/evals.json — nothing to run. See skill-creator's references/evaluation.md.`);
  }
  try {
    return JSON.parse(readFileSync(skill.suitePath, "utf8")) as Suite;
  } catch (error) {
    throw new Error(`${skill.suitePath} is not valid JSON: ${(error as Error).message}`);
  }
}

// ── check: the half that is a gate ──────────────────────────────────────────────────

/**
 * Validate one suite. Returns the problems found, so a caller can report all of them at
 * once — a gate that stops at the first problem makes the reader run it N times.
 *
 * Every rule here exists because the thing it checks went wrong somewhere real:
 *   - two suites using two different schemas (`skill`+`cases` vs `skill_name`+`evals`),
 *     which is why the canonical keys are now a FAILURE rather than something to tolerate;
 *   - a suite whose assertions are all unverifiable prose, which grades as all-PASS;
 *   - trigger tests made of obviously-irrelevant prompts, which any description passes.
 */
export function checkSuite(skill: Skill, suite: Suite): { problems: string[]; warnings: string[] } {
  const problems: string[] = [];
  const warnings: string[] = [];
  const at = (msg: string) => problems.push(`${skill.name}: ${msg}`);
  const warn = (msg: string) => warnings.push(`${skill.name}: ${msg}`);

  for (const key of Object.keys(suite)) {
    if (!(KNOWN_TOP_KEYS as readonly string[]).includes(key)) {
      at(`unknown top-level key '${key}'. Known: ${KNOWN_TOP_KEYS.join(", ")}`);
    }
  }
  // The rename that caused the split, called out by name so the message names the fix.
  for (const [legacy, canonical] of [["skill", "skill_name"], ["cases", "evals"]] as const) {
    if (legacy in suite && !(canonical in suite)) {
      at(`uses '${legacy}'; the canonical key is '${canonical}' (suites disagreed on this until 2026-09-17)`);
    }
  }
  if (!suite.skill_name) at("no 'skill_name'");
  else if (suite.skill_name !== skill.name) at(`'skill_name' is '${suite.skill_name}' but the directory is '${skill.name}'`);

  // The RED phase is the discipline the whole method rests on, and a suite without one
  // cannot be trusted to have been written against a real failure. Reported, not failed:
  // skill-creator's own suite predates the rule, and a gate that fails its own author's
  // suite on the day it lands is a gate nobody keeps.
  if (!suite.baseline) {
    warn("no 'baseline' recorded — the method's RED phase. Run the task without the skill first and record the failure.");
  }

  if (!Array.isArray(suite.evals)) {
    at("no 'evals' array");
    return { problems, warnings };
  }
  if (suite.evals.length === 0) at("'evals' is empty");

  const seen = new Set<string>();
  suite.evals.forEach((c, i) => {
    const where = `evals[${i}]`;
    if (c.id === undefined || c.id === null || c.id === "") at(`${where}: no 'id'`);
    else {
      const id = String(c.id);
      if (seen.has(id)) at(`${where}: duplicate id '${id}'`);
      seen.add(id);
    }
    if (typeof c.prompt !== "string" || !c.prompt.trim()) at(`${where}: 'prompt' is empty`);
    if (!Array.isArray(c.assertions) || c.assertions.length === 0) {
      at(`${where}: no assertions — an ungradeable case reports as a pass`);
    } else if (c.assertions.some((a) => typeof a !== "string" || !a.trim())) {
      at(`${where}: an assertion is empty`);
    }
    for (const f of c.files ?? []) {
      // Resolved against the skill dir, which is what the method doc's fixture paths mean.
      if (!existsSync(join(skill.dir, f))) at(`${where}: fixture '${f}' does not exist`);
    }
  });

  const tt = suite.trigger_tests;
  if (tt) {
    if (!Array.isArray(tt.should_trigger) || tt.should_trigger.length === 0) {
      at("trigger_tests has no 'should_trigger'");
    }
    if (!Array.isArray(tt.should_not_trigger) || tt.should_not_trigger.length === 0) {
      at("trigger_tests has no 'should_not_trigger'");
    }
    // The near-miss rule: a prompt any description would decline tests nothing. A
    // parenthetical naming the sibling or the reason is how these suites mark a near-miss,
    // so its absence is worth a look — reported, not failed, because the phrasing is a
    // judgement and the rule's whole point is that a judgement be visible.
    for (const p of tt.should_not_trigger ?? []) {
      if (!/[(\u2014]/.test(p) && !/near-miss|sibling|out of scope/i.test(p)) {
        warn(
          `trigger_tests.should_not_trigger entry looks obviously-irrelevant, so it tests nothing: "${p.slice(0, 60)}"\n      Name the near-miss sibling or say why it is a boundary case.`,
        );
      }
    }
  }
  return { problems, warnings };
}

function checkAll(only?: string): boolean {
  const skills = only ? [findSkill(only)] : allSkills();
  const withSuites = skills.filter((s) => s.suitePath);
  let failed = 0;
  let warned = 0;
  for (const skill of withSuites) {
    const { problems, warnings } = checkSuite(skill, readSuite(skill));
    if (warnings.length) {
      warned += 1;
      console.log(`warn ${skill.name}`);
      for (const w of warnings) console.log(`    ${w.replace(/^[^:]+: /, "")}`);
    }
    if (problems.length) {
      failed += 1;
      console.error(`FAIL ${skill.name}`);
      for (const p of problems) console.error(`    ${p}`);
    }
  }
  const missing = skills.filter((s) => !s.suitePath);
  console.log(
    `skill-eval: ${withSuites.length - failed}/${withSuites.length} suite(s) valid, ${warned} warned` +
      `, ${missing.length} skill(s) with no suite` +
      (missing.length ? ` (${missing.map((s) => s.name).join(", ")})` : ""),
  );
  return failed === 0;
}

// ── run: the half that costs money ──────────────────────────────────────────────────

type Arm = "with_skill" | "without_skill";

type ClaudeRun = {
  result: string;
  durationMs: number;
  tokensIn: number;
  tokensOut: number;
  cacheRead: number;
  costUsd: number;
  model: string;
};

/**
 * One non-interactive Claude Code run.
 *
 * stderr is NOT merged into stdout: the JSON event array must parse, and a warning on
 * stderr ahead of it makes the whole payload unparseable — which is how a first attempt at
 * this returned an empty result that looked like a silent model failure.
 */
function claude(prompt: string, cwd: string, model: string | null): ClaudeRun {
  const args = ["-p", prompt, "--output-format", "json"];
  if (model) args.push("--model", model);
  const proc = Bun.spawnSync(["claude", ...args], { cwd, stdout: "pipe", stderr: "pipe" });
  const stdout = proc.stdout.toString();
  let events: unknown;
  try {
    events = JSON.parse(stdout);
  } catch {
    throw new Error(
      `claude produced no parseable JSON (exit ${proc.exitCode}).\n` +
        `  stderr: ${proc.stderr.toString().slice(0, 400)}\n  stdout: ${stdout.slice(0, 200)}`,
    );
  }
  const list = Array.isArray(events) ? events : [events];
  const result = [...list].reverse().find((e: any) => e?.type === "result") as any;
  if (!result) throw new Error("claude emitted no result event");
  if (result.is_error) throw new Error(`claude reported an error: ${String(result.result).slice(0, 300)}`);
  const usage = result.usage ?? {};
  const init = list.find((e: any) => e?.type === "system" && e?.subtype === "init") as any;
  return {
    result: String(result.result ?? ""),
    durationMs: result.duration_ms ?? 0,
    tokensIn: usage.input_tokens ?? 0,
    tokensOut: usage.output_tokens ?? 0,
    cacheRead: usage.cache_read_input_tokens ?? 0,
    costUsd: result.total_cost_usd ?? 0,
    model: init?.model ?? "unknown",
  };
}

/** Grades one arm's output against the case's assertions, requiring quoted evidence. */
function grade(
  casePrompt: string,
  assertions: string[],
  output: string,
  cwd: string,
  model: string | null,
): { assertion: string; verdict: string; evidence: string }[] {
  const prompt = [
    "You are grading one agent output against a list of assertions, for a skill evaluation.",
    "",
    "THE TASK THE AGENT WAS GIVEN:",
    casePrompt,
    "",
    "THE AGENT'S OUTPUT:",
    output.slice(0, 20000),
    "",
    "ASSERTIONS TO GRADE, one by one:",
    ...assertions.map((a, i) => `${i + 1}. ${a}`),
    "",
    "Rules:",
    "- PASS only if the output demonstrably satisfies the assertion. Quote the exact text that",
    "  satisfies or violates it in `evidence`. A section TITLED 'Summary' containing one vague",
    "  sentence does NOT satisfy 'includes a summary' — the label is not the substance.",
    "- FAIL when the evidence is absent, or when you would have to infer that the agent meant it.",
    "- Do not reward effort, length, or confidence.",
    "",
    "Reply with ONLY a JSON array, no prose and no code fence, one object per assertion:",
    '[{"assertion": "<the assertion text verbatim>", "verdict": "PASS" | "FAIL", "evidence": "<quote or the reason it is absent>"}]',
  ].join("\n");

  const run = claude(prompt, cwd, model);
  const text = run.result.trim().replace(/^```(?:json)?\n?/, "").replace(/\n?```$/, "");
  const start = text.indexOf("[");
  const end = text.lastIndexOf("]");
  if (start === -1 || end === -1) {
    return assertions.map((a) => ({ assertion: a, verdict: "UNGRADED", evidence: text.slice(0, 200) }));
  }
  try {
    return JSON.parse(text.slice(start, end + 1));
  } catch {
    return assertions.map((a) => ({ assertion: a, verdict: "UNGRADED", evidence: "grader returned unparseable JSON" }));
  }
}

function nextIteration(workspace: string): number {
  if (!existsSync(workspace)) return 1;
  const nums = readdirSync(workspace)
    .map((d) => /^iteration-(\d+)$/.exec(d)?.[1])
    .filter(Boolean)
    .map(Number);
  return nums.length ? Math.max(...nums) + 1 : 1;
}

function runEval(skillName: string, flags: { caseId?: string; model?: string | null; gradeModel?: string | null; noGrade?: boolean }) {
  const skill = findSkill(skillName);
  const suite = readSuite(skill);
  const { problems } = checkSuite(skill, suite);
  if (problems.length) {
    console.error(`skill-eval: ${skillName}'s suite is invalid, refusing to run it:`);
    for (const p of problems) console.error(`    ${p}`);
    process.exitCode = 1;
    return;
  }
  const cases = (suite.evals ?? []).filter((c) => !flags.caseId || String(c.id) === flags.caseId);
  if (!cases.length) throw new Error(`no case matching '${flags.caseId}'`);

  const workspace = join(dirname(skill.dir), `${skill.name}-workspace`);
  const iteration = join(workspace, `iteration-${nextIteration(workspace)}`);
  mkdirSync(iteration, { recursive: true });
  console.log(`skill-eval: ${skill.name} -> ${iteration.replace(SJEL_ROOT + "/", "")}`);

  const arms: Arm[] = ["with_skill", "without_skill"];
  const benchmark: any[] = [];

  for (const c of cases) {
    const slug = `eval-${String(c.id).replace(/[^a-z0-9]+/gi, "-").toLowerCase()}`;
    const caseDir = join(iteration, slug);
    const row: any = { id: c.id, assertions: c.assertions.length };

    for (const arm of arms) {
      // A throwaway project dir. `with_skill` gets the skill at project level; the baseline
      // gets no `.claude/skills/` at all, which is the one difference between the arms.
      const armDir = join(caseDir, arm);
      rmSync(armDir, { recursive: true, force: true });
      mkdirSync(join(armDir, "outputs"), { recursive: true });
      if (arm === "with_skill") {
        cpSync(skill.dir, join(armDir, ".claude", "skills", skill.name), { recursive: true });
        // The suite's own fixtures are not part of the skill; keep them out of the treatment.
        rmSync(join(armDir, ".claude", "skills", skill.name, "evals"), { recursive: true, force: true });
      }
      for (const f of c.files ?? []) {
        const dest = join(armDir, f);
        mkdirSync(dirname(dest), { recursive: true });
        cpSync(join(skill.dir, f), dest);
      }

      const prompt = [
        c.prompt,
        "",
        `(Work in the current directory. Write any files you produce here.)`,
      ].join("\n");
      const t0 = Bun.nanoseconds();
      const run = claude(prompt, armDir, flags.model ?? null);
      const wall = Math.round((Bun.nanoseconds() - t0) / 1e6);

      writeFileSync(join(armDir, "outputs", "result.txt"), run.result + "\n");
      writeFileSync(
        join(armDir, "timing.json"),
        JSON.stringify(
          {
            total_tokens: run.tokensIn + run.tokensOut + run.cacheRead,
            output_tokens: run.tokensOut,
            cache_read_input_tokens: run.cacheRead,
            duration_ms: run.durationMs,
            wall_ms: wall,
            cost_usd: run.costUsd,
            model: run.model,
          },
          null,
          2,
        ) + "\n",
      );

      let graded: any[] = [];
      if (!flags.noGrade) {
        graded = grade(c.prompt, c.assertions, run.result, armDir, flags.gradeModel ?? null);
        writeFileSync(join(armDir, "grading.json"), JSON.stringify(graded, null, 2) + "\n");
      }
      const passed = graded.filter((g) => g.verdict === "PASS").length;
      row[arm] = {
        pass: graded.length ? passed : null,
        of: graded.length || null,
        cost_usd: Number(run.costUsd.toFixed(4)),
        duration_ms: run.durationMs,
        output_tokens: run.tokensOut,
      };
      console.log(
        `  ${slug} [${arm}] ` +
          (graded.length ? `${passed}/${graded.length} asserted` : "(ungraded)") +
          ` $${run.costUsd.toFixed(2)} ${run.durationMs}ms`,
      );
    }

    const w = row.with_skill, wo = row.without_skill;
    if (w.pass !== null && wo.pass !== null) {
      row.delta_pass = w.pass - wo.pass;
      row.delta_cost_usd = Number((w.cost_usd - wo.cost_usd).toFixed(4));
    }
    benchmark.push(row);
    console.log(
      `  -> delta: ${row.delta_pass === undefined ? "n/a" : `${row.delta_pass > 0 ? "+" : ""}${row.delta_pass} assertions`}` +
        (row.delta_cost_usd === undefined ? "" : `, $${row.delta_cost_usd > 0 ? "+" : ""}${row.delta_cost_usd}`),
    );
  }

  const graded = benchmark.filter((r) => r.delta_pass !== undefined);
  const summary = {
    skill: skill.name,
    iteration: iteration.replace(SJEL_ROOT + "/", ""),
    record: "2026-09-17",
    model: flags.model ?? "(session default)",
    grade_model: flags.gradeModel ?? "(session default)",
    arms: "with_skill = project-level .claude/skills/<name>; without_skill = same dir, no skills",
    delta_basis: "the skill added to this machine's real install; the other installed skills load in BOTH arms",
    totals: graded.length
      ? {
          with_skill_pass: graded.reduce((n, r) => n + r.with_skill.pass, 0),
          without_skill_pass: graded.reduce((n, r) => n + r.without_skill.pass, 0),
          of: graded.reduce((n, r) => n + r.with_skill.of, 0),
          cost_usd: Number(benchmark.reduce((n, r) => n + r.with_skill.cost_usd + r.without_skill.cost_usd, 0).toFixed(4)),
        }
      : null,
    cases: benchmark,
  };
  writeFileSync(join(iteration, "benchmark.json"), JSON.stringify(summary, null, 2) + "\n");
  console.log(`skill-eval: wrote ${join(iteration, "benchmark.json").replace(SJEL_ROOT + "/", "")}`);
  if (summary.totals) {
    const t = summary.totals;
    console.log(
      `skill-eval: pass ${t.with_skill_pass} with skill vs ${t.without_skill_pass} without, of ${t.of} assertions; $${t.cost_usd}`,
    );
  }
}

// ── CLI ─────────────────────────────────────────────────────────────────────────────

const HELP = `tools/skill-eval — run skill evals, and check the suites.

  tools/skill-eval list                     every suite, its case count, whether a baseline is recorded
  tools/skill-eval check [<skill>]          validate the suite(s). This half runs in CI.
  tools/skill-eval run <skill> [flags]      execute the A/B and write a benchmark

Flags for run:
  --case <id>          one case instead of all of them (use this while iterating; a graded
                       case is four model calls)
  --model <name>       pin the model for the task runs
  --grade-model <name> pin the model for grading (a cheaper one is usually right)
  --no-grade           run the tasks only, skip grading — cheapest way to read raw outputs

A graded case costs roughly $1.50 on this machine. See the header of tools/skill-eval.ts for
what the delta does and does not measure.`;

// Guarded so this module can be imported for its checker without the CLI running itself
// on import — tools/skill-eval.test.ts does exactly that, and tools/lib/ci-workflow.ts was
// split out of ci-local.ts for the same reason.
if (import.meta.main) {
const argv = process.argv.slice(2);
const flag = (name: string): string | undefined => {
  const i = argv.indexOf(name);
  return i === -1 ? undefined : argv[i + 1];
};
const positional = argv.filter((a) => !a.startsWith("--"));
const command = positional[0] ?? "list";

if (argv.includes("-h") || argv.includes("--help")) {
  console.log(HELP);
  process.exit(0);
}

try {
  if (command === "list") {
    const skills = allSkills();
    const withSuites = skills.filter((s) => s.suitePath);
    for (const s of withSuites) {
      const suite = readSuite(s);
      const base = suite.baseline ? "baseline recorded" : "NO baseline recorded";
      const trig = suite.trigger_tests ? `, ${suite.trigger_tests.should_trigger?.length ?? 0}/${suite.trigger_tests.should_not_trigger?.length ?? 0} trigger tests` : "";
      console.log(`  ${s.name.padEnd(20)} ${String(suite.evals?.length ?? 0).padStart(2)} case(s)  ${base}${trig}  [${s.pack}]`);
    }
    const missing = skills.filter((s) => !s.suitePath);
    console.log(`\n${withSuites.length} suite(s), ${missing.length} of ${skills.length} skills with none.`);
    console.log("Add one with skill-creator's references/evaluation.md — 2-3 cases, assertions written AFTER the first run.");
  } else if (command === "check") {
    const only = positional[1];
    if (!checkAll(only === "--all" ? undefined : only)) process.exitCode = 1;
  } else if (command === "run") {
    const name = positional[1];
    if (!name) throw new Error("run needs a skill name");
    runEval(name, {
      caseId: flag("--case"),
      model: flag("--model") ?? null,
      gradeModel: flag("--grade-model") ?? null,
      noGrade: argv.includes("--no-grade"),
    });
  } else {
    console.error(`skill-eval: unknown command '${command}'`);
    console.error(HELP);
    process.exitCode = 2;
  }
} catch (error) {
  console.error(`skill-eval: ${(error as Error).message}`);
  process.exitCode = 1;
}
}
