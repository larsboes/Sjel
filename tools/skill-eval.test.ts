// tools/skill-eval.test.ts — the suite checker, against planted suites.
//
// Only the checker is tested here, and that is the honest boundary: `run` costs about $1.50 a
// case and depends on a model, so it cannot be a test. What CAN be proven cheaply is that the
// checker fails the shapes that matter — which is the half wired into CI.
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
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { checkSuite, type Skill, type Suite } from "./skill-eval.ts";

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
