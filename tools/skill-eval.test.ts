// tools/skill-eval.test.ts — the suite checker, against planted suites.
//
// Suite checks and runner accounting are tested offline. Runner tests inject deterministic
// model outputs; they never invoke a model or the network.
//
// Every case below is a shape that either happened or would silently pass:
//   - the two schemas the real suites used before 2026-09-17 (`skill`+`cases` vs
//     `skill_name`+`evals`), which is why that is a named failure and not a tolerated alias;
//   - a case with no assertions, which grades as a pass because there is nothing to fail;
//   - a fixture path pointing at a file that is not there;
//   - a trigger test made of obviously-irrelevant prompts, which any description passes.
//
// Run: bun test tools/skill-eval.test.ts

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { checkSuite, executeCases, parseGrades, parseRunFlags, recordedCost, sampleDirectory, sampleStats, summarizeBenchmark, type ArmRecord, type ClaudeRun, type SampleRecord, type Skill, type Suite } from "./skill-eval.ts";

let root: string;
let skill: Skill;

/** A skill on disk with an empty references/ so a fixture path can be planted. */
function plantSkill(fixtures: string[] = []): void {
  const dir = join(root, "writing", "skills", "demo-skill");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "SKILL.md"), "---\nname: demo-skill\ndescription: does a thing\n---\n\nBody.\n");
  for (const f of fixtures) {
    mkdirSync(join(dir, f, ".."), { recursive: true });
    writeFileSync(join(dir, f), "fixture\n");
  }
  skill = { name: "demo-skill", pack: "writing", dir, suitePath: join(dir, "evals", "evals.json") };
}

function check(suite: unknown) {
  return checkSuite(skill, suite as Suite);
}

const ok = {
  skill_name: "demo-skill",
  baseline: { captured: "2026-09-17", method: "ran it without the skill", failure: "did the thing badly" },
  evals: [{ id: 1, prompt: "do the thing", assertions: ["it did the thing"] }],
};

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "sjel-skill-eval-test-"));
  plantSkill();
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("recordedCost", () => {
  test("accepts explicit charges and rejects unknown or invalid billing", () => {
    expect(recordedCost(0)).toBe(0);
    expect(recordedCost(0.25)).toBe(0.25);
    for (const value of [undefined, null, "0.25", -1, NaN, Infinity]) {
      expect(() => recordedCost(value)).toThrow("cost is unknown");
    }
  });
});

describe("sampleStats", () => {
  test("no data and one observation do not invent uncertainty", () => {
    expect(sampleStats([])).toBeNull();
    expect(sampleStats([7])).toEqual({ n: 1, mean: 7, stddev: null, min: 7, max: 7 });
  });
  test("uses sample rather than population standard deviation", () => {
    const stats = sampleStats([1, 2, 3, 4])!;
    expect(stats.n).toBe(4);
    expect(stats.mean).toBe(2.5);
    expect(stats.stddev).toBeCloseTo(Math.sqrt(5 / 3), 12);
    expect(stats.min).toBe(1);
    expect(stats.max).toBe(4);
    expect(sampleStats([0, 0])!.stddev).toBe(0);
    expect(sampleStats([-2, -2])!.stddev).toBe(0);
    expect(sampleStats([Number.MAX_VALUE, Number.MAX_VALUE])!.mean).toBe(Number.MAX_VALUE);
  });
  test("rejects non-finite samples rather than silently dropping them", () => {
    for (const invalid of [NaN, Infinity, -Infinity]) {
      expect(() => sampleStats([1, invalid])).toThrow("finite");
    }
  });
});

describe("parseRunFlags", () => {
  test("repetition is opt-in and other flags retain their values", () => {
    expect(parseRunFlags([])).toEqual({ repeat: 1 });
    expect(parseRunFlags(["--repeat", "3", "--case", "c1", "--model", "task", "--grade-model", "judge", "--no-grade"]))
      .toEqual({ repeat: 3, caseId: "c1", model: "task", gradeModel: "judge", noGrade: true });
    expect(parseRunFlags(["--repeat", String(Number.MAX_SAFE_INTEGER)]).repeat).toBe(Number.MAX_SAFE_INTEGER);
  });
  test("bad, missing, unknown, duplicate and stray options fail before execution", () => {
    for (const args of [
      ["--repeat"], ["--repeat", "0"], ["--repeat", "-1"], ["--repeat", "1.5"],
      ["--repeat", "NaN"], ["--repeat", "Infinity"], ["--repeat", "1e3"],
      ["--repeat", "9007199254740992"], ["--repeat", "2", "--repeat", "3"],
      ["--case"], ["--model"], ["--grade-model", "--no-grade"], ["--model", " "],
      ["--unknown"], ["--no-grade", "true"], ["extra"], ["--repeat=2"],
    ]) expect(() => parseRunFlags(args)).toThrow();
  });
  test("invalid CLI options are rejected before looking up a skill", () => {
    const result = Bun.spawnSync([process.execPath, join(import.meta.dir, "skill-eval.ts"), "run", "nonexistent-skill", "--repeat", "0"]);
    expect(result.exitCode).toBe(1);
    expect(result.stderr.toString()).toContain("positive safe integer");
    expect(result.stderr.toString()).not.toContain("no skill named");
  });
});

describe("exact grader coverage", () => {
  const a = { assertion: "a", verdict: "PASS", evidence: "quoted text" };
  const b = { assertion: "b", verdict: "FAIL", evidence: "absent" };
  test("accepts all assertions once in any order, including fenced JSON", () => {
    expect(parseGrades(["a", "b"], JSON.stringify([b, a]))).toEqual([b, a]);
    expect(parseGrades(["a"], "```json\n" + JSON.stringify([a]) + "\n```")).toEqual([a]);
  });
  test("malformed or partial payloads invalidate the entire grade", () => {
    for (const payload of [[], [a], [a, a], [a, b, b], [a, { ...b, assertion: "unknown" }],
      [a, { ...b, verdict: "pass" }], [a, { ...b, evidence: " " }], [a, { ...b, evidence: 3 }],
      [a, null], [a, {}], { grades: [a, b] }]) {
      const grades = parseGrades(["a", "b"], JSON.stringify(payload));
      expect(grades.map((g) => g.verdict)).toEqual(["UNGRADED", "UNGRADED"]);
      expect(grades.every((g) => g.evidence.includes("Incomplete grading"))).toBe(true);
    }
    expect(parseGrades(["a"], "not JSON")[0].verdict).toBe("UNGRADED");
    expect(parseGrades(["a", "a"], JSON.stringify([a, a])).every((g) => g.verdict === "UNGRADED")).toBe(true);
  });
});

function mockRun(result: string, costUsd = 0.25): ClaudeRun {
  return { result, costUsd, durationMs: 12, tokensIn: 2, tokensOut: 3, cacheRead: 4, model: "mock" };
}

describe("offline runner", () => {
  test("repeats have fresh workspaces, explicit identities and all four calls counted", () => {
    const dirs: string[] = [];
    const models: (string | null)[] = [];
    const summary = executeCases(skill, ok.evals, { repeat: 2, model: "task", gradeModel: "judge" }, (_prompt, cwd, model) => {
      models.push(model);
      if (model === "task") {
        expect(existsSync(join(cwd, "marker"))).toBe(false);
        writeFileSync(join(cwd, "marker"), "seen");
        dirs.push(cwd);
      }
      return mockRun(model === "judge" ? JSON.stringify([{ assertion: "it did the thing", verdict: "PASS", evidence: "thing" }]) : "thing");
    });
    expect(models).toEqual(["task", "judge", "task", "judge", "task", "judge", "task", "judge"]);
    expect(new Set(dirs).size).toBe(4);
    expect(summary.cases.map((r) => r.sample)).toEqual([1, 2]);
    expect(summary.totals).toMatchObject({ task_cost_usd: 1, grading_cost_usd: 1, cost_usd: 2 });
    expect(summary.totals.with_skill.cost_usd).toBe(1);
    expect(summary.record).toBe(new Date().toISOString().slice(0, 10));
    expect(summary.statistics_note).toContain("not isolated");
    const stats = summary.statistics[0].with_skill as { pass_rate: { n: number; mean: number } };
    expect(stats.pass_rate).toMatchObject({ n: 2, mean: 1 });
    for (const cwd of dirs) {
      expect(JSON.parse(readFileSync(join(cwd, "sample.json"), "utf8")).sample).toBe(cwd.includes("sample-1") ? 1 : 2);
      const grading = JSON.parse(readFileSync(join(cwd, "grading-timing.json"), "utf8"));
      expect(grading).toMatchObject({ total_tokens: 9, duration_ms: 12, cost_usd: 0.25, model: "mock" });
      expect(grading.wall_ms).toBeGreaterThanOrEqual(0);
    }
  });
  test("incomplete grading retains artifacts and costs, but no pass-rate observation", () => {
    const summary = executeCases(skill, ok.evals, { repeat: 1 }, () => mockRun("[]"));
    expect(summary.complete).toBe(false);
    expect(summary.totals).toMatchObject({ task_cost_usd: 0.5, grading_cost_usd: 0.5, cost_usd: 1 });
    expect(summary.totals.with_skill).toMatchObject({ incomplete: 1, pass: 0, of: 0 });
    expect(summary.statistics[0].with_skill).toMatchObject({ pass_rate: null });
    expect(summary.cases[0].with_skill).toMatchObject({ grading: "incomplete", pass: null, of: null });
    expect(summary.cases[0].delta_pass).toBeUndefined();
    const armDir = join(summary.iteration, "eval-1", "with_skill");
    expect(existsSync(join(armDir, "outputs", "result.txt"))).toBe(true);
    expect(JSON.parse(readFileSync(join(armDir, "grading.json"), "utf8"))[0].verdict).toBe("UNGRADED");
    expect(existsSync(join(summary.iteration, "benchmark.json"))).toBe(true);
  });
  test("no-grade is intentionally ungraded and keeps task costs", () => {
    let calls = 0;
    const summary = executeCases(skill, ok.evals, { repeat: 1, noGrade: true }, () => { calls++; return mockRun("output"); });
    expect(calls).toBe(2);
    expect(summary.complete).toBe(true);
    expect(summary.cases[0].with_skill.grading).toBe("skipped");
    expect(summary.totals).toMatchObject({ task_cost_usd: 0.5, grading_cost_usd: 0, cost_usd: 0.5 });
    expect(summary.statistics[0].with_skill).toMatchObject({ pass_rate: null, grading_duration_ms: null });
    expect(existsSync(join(summary.iteration, "eval-1", "with_skill", "grading.json"))).toBe(false);
    expect(sampleDirectory("iteration-1", "c1", 1, 1)).toBe(join("iteration-1", "eval-c1"));
  });
  test("statistics stay per case and arm and skip missing rates", () => {
    const good: ArmRecord = { grading: "complete", pass: 1, of: 1, task_cost_usd: 1, grading_cost_usd: 2, cost_usd: 3, duration_ms: 10, grading_duration_ms: 20, output_tokens: 3 };
    const missing: ArmRecord = { ...good, grading: "incomplete", pass: null, of: null };
    const row = (id: string, sample: number, with_skill: ArmRecord, without_skill = good): SampleRecord => ({ id, sample, assertions: 1, with_skill, without_skill });
    const summary = summarizeBenchmark([row("a", 1, good), row("a", 2, missing), row("b", 1, { ...good, pass: 0 })]);
    expect(summary.statistics).toHaveLength(2);
    expect(summary.statistics[0].with_skill).toMatchObject({ pass_rate: { n: 1, mean: 1, stddev: null }, cost_usd: { n: 2 } });
    expect(summary.statistics[1].with_skill).toMatchObject({ pass_rate: { n: 1, mean: 0 } });
    expect(summary.statistics[1].without_skill).toMatchObject({ pass_rate: { n: 1, mean: 1 } });
    expect(summary.totals.cost_usd).toBe(18);
    expect(summary.totals.with_skill.of).toBe(2);
    expect(summary.complete).toBe(false);
  });
});

describe("checkSuite — the green path", () => {
  test("a well-formed suite has neither problems nor warnings", () => {
    expect(check(ok)).toEqual({ problems: [], warnings: [] });
  });

  test("string ids and the optional extras are fine", () => {
    const r = check({
      ...ok,
      note: "free text",
      results: { recorded: "2026-09-17" },
      trigger_tests: {
        should_trigger: ["do the thing"],
        should_not_trigger: ["do the other thing (near-miss: other-skill)"],
      },
      evals: [{ id: "c1-do-the-thing", prompt: "do the thing", expected_output: "the thing", assertions: ["it did the thing"] }],
    });
    expect(r).toEqual({ problems: [], warnings: [] });
  });
});

describe("checkSuite — the schema split stays split", () => {
  // The real failure: skill-creator used skill_name+evals, slide-deck used skill+cases, and
  // nothing read either, so the divergence survived unnoticed. A tolerated alias would let it
  // come straight back.
  test("the old keys are named as the fix, not accepted", () => {
    const { problems } = check({ skill: "demo-skill", cases: [] });
    expect(problems.join("\n")).toContain("the canonical key is 'skill_name'");
    expect(problems.join("\n")).toContain("the canonical key is 'evals'");
  });

  test("an unknown top-level key is a problem, so a typo cannot hide", () => {
    const { problems } = check({ ...ok, assertion: ["typo'd"] });
    expect(problems.join("\n")).toContain("unknown top-level key 'assertion'");
  });

  test("skill_name must match the directory", () => {
    const { problems } = check({ ...ok, skill_name: "other-name" });
    expect(problems.join("\n")).toContain("the directory is 'demo-skill'");
  });
});

describe("checkSuite — ungradeable cases", () => {
  // A case with no assertions cannot fail, so it reports as a pass and inflates the benchmark.
  test("no assertions is a problem, and says why", () => {
    const { problems } = check({ ...ok, evals: [{ id: 1, prompt: "do the thing", assertions: [] }] });
    expect(problems.join("\n")).toContain("no assertions");
    expect(problems.join("\n")).toContain("reports as a pass");
  });

  test("a missing assertions field is the same problem", () => {
    const { problems } = check({ ...ok, evals: [{ id: 1, prompt: "do the thing" }] });
    expect(problems.join("\n")).toContain("no assertions");
  });

  test("duplicate ids are caught", () => {
    const { problems } = check({
      ...ok,
      evals: [
        { id: 1, prompt: "a", assertions: ["x"] },
        { id: 1, prompt: "b", assertions: ["y"] },
      ],
    });
    expect(problems.join("\n")).toContain("duplicate id '1'");
  });

  test("an empty prompt is caught", () => {
    const { problems } = check({ ...ok, evals: [{ id: 1, prompt: "   ", assertions: ["x"] }] });
    expect(problems.join("\n")).toContain("'prompt' is empty");
  });
});

describe("checkSuite — fixtures resolve", () => {
  test("a fixture that is not there is a problem", () => {
    const { problems } = check({ ...ok, evals: [{ id: 1, prompt: "p", assertions: ["x"], files: ["evals/files/absent.csv"] }] });
    expect(problems.join("\n")).toContain("fixture 'evals/files/absent.csv' does not exist");
  });

  test("a fixture that is there is fine", () => {
    plantSkill(["evals/files/present.csv"]);
    const { problems } = check({ ...ok, evals: [{ id: 1, prompt: "p", assertions: ["x"], files: ["evals/files/present.csv"] }] });
    expect(problems).toEqual([]);
  });
});

describe("checkSuite — the RED phase is reported, not enforced", () => {
  // A suite with no baseline cannot be trusted to have been written against a real failure.
  // It warns rather than fails: skill-creator's own suite predates the rule, and a gate that
  // fails its author's suite on the day it lands is a gate nobody keeps.
  test("a suite with no baseline warns and does not fail", () => {
    const { problems, warnings } = check({ skill_name: "demo-skill", evals: ok.evals });
    expect(problems).toEqual([]);
    expect(warnings.join("\n")).toContain("no 'baseline' recorded");
  });
});

describe("checkSuite — trigger tests must be near-misses", () => {
  test("an obviously-irrelevant should_not_trigger warns", () => {
    const { problems, warnings } = check({
      ...ok,
      trigger_tests: { should_trigger: ["do the thing"], should_not_trigger: ["what is the weather in Paris"] },
    });
    expect(problems).toEqual([]);
    expect(warnings.join("\n")).toContain("looks obviously-irrelevant");
  });

  test("a near-miss naming its sibling does not", () => {
    const { warnings } = check({
      ...ok,
      trigger_tests: { should_trigger: ["do the thing"], should_not_trigger: ["do the other thing (near-miss: other-skill)"] },
    });
    expect(warnings).toEqual([]);
  });

  test("an empty should_not_trigger is a problem, not a warning", () => {
    // This is a fact, not a judgement: with nothing to decline, tier 1 tests nothing at all.
    const { problems } = check({ ...ok, trigger_tests: { should_trigger: ["do the thing"], should_not_trigger: [] } });
    expect(problems.join("\n")).toContain("no 'should_not_trigger'");
  });
});
