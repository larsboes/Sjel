// tools/ci-local.test.ts — the parser and the local-safety verdict, on planted workflows.
//
// The end-to-end behaviour (does it actually run the gates, does it actually refuse a red
// one) is tools/ci-local.test.sh's job. This covers the two decisions that have no output
// of their own: which steps of a job are work, and which jobs may touch this machine.
// Run: bun test tools/ci-local.test.ts

import { describe, expect, test } from "bun:test";
import { REFUSED, RUNS_HERE, UNCLASSIFIED, parseJobs } from "./lib/ci-workflow.ts";

const workflow = (body: string) => `name: CI\non: [push]\njobs:\n${body}`;

describe("parseJobs", () => {
  test("a uses: step is not work, and does not count as a step", () => {
    const jobs = parseJobs(
      workflow(`  repo-gates:
    runs-on: ubuntu-latest
    steps:
      - name: Check out Sjel
        uses: actions/checkout@v5
      - name: Publication hygiene
        run: tools/check-publication-hygiene.sh
`),
    );
    expect(jobs).toHaveLength(1);
    expect(jobs[0].steps).toHaveLength(1);
    expect(jobs[0].steps[0].name).toBe("Publication hygiene");
  });

  // Dropping it silently would under-report the job, which is the exact failure the tool
  // exists to remove: a runner reporting success over work it never did.
  test("a step with neither run: nor uses: is a malformed workflow, not a skip", () => {
    expect(() =>
      parseJobs(
        workflow(`  repo-gates:
    steps:
      - name: nothing at all
`),
      ),
    ).toThrow("repo-gates: step 1 has neither run: nor uses:");
  });

  test("an unnamed step is titled by the first line of its script", () => {
    const jobs = parseJobs(
      workflow(`  repo-gates:
    steps:
      - run: |
          tools/check-publication-hygiene.sh
          echo done
`),
    );
    expect(jobs[0].steps[0].name).toBe("tools/check-publication-hygiene.sh");
    expect(jobs[0].steps[0].script).toContain("echo done");
  });

  // working-directory was dropped here until 2026-09-17, so ci-local ran such a step from
  // the checkout root and reported a failure CI would never produce — a step that looked
  // broken rather than mis-run. Carrying it is what makes the replay faithful.
  test("a step's working-directory is carried, not dropped", () => {
    const jobs = parseJobs(
      workflow(`  bun-tests:
    steps:
      - name: vendored package tests
        working-directory: Packs/harness/pi-packages/accordion
        run: bun run test
`),
    );
    expect(jobs[0].steps[0].workingDirectory).toBe("Packs/harness/pi-packages/accordion");
  });

  test("a step without one carries no workingDirectory, rather than the empty string", () => {
    const jobs = parseJobs(
      workflow(`  repo-gates:
    steps:
      - name: Publication hygiene
        run: tools/check-publication-hygiene.sh
`),
    );
    // Absent, not "": runStep branches on truthiness, and "" would join to the root anyway —
    // but an absent field is what the type says and what a reader should find.
    expect("workingDirectory" in jobs[0].steps[0]).toBe(false);
  });

  test("refuses a classified job when execution semantics cannot be replayed", () => {
    for (const setting of ["if: false", "env: { MODE: ci }", "shell: python", "continue-on-error: true"]) {
      const jobs = parseJobs(workflow(`  repo-gates:\n    steps:\n      - run: echo ok\n        ${setting}\n`));
      expect(jobs[0].refusal).toContain("cannot be replayed faithfully");
    }
    const expression = parseJobs(workflow("  repo-gates:\n    steps:\n      - run: echo '${{ github.ref }}'\n"));
    expect(expression[0].refusal).toContain("cannot be replayed faithfully");
    const environment = parseJobs("env: { MODE: ci }\n" + workflow("  repo-gates:\n    steps:\n      - run: echo ok\n"));
    expect(environment[0].refusal).toContain("cannot be replayed faithfully");
  });

  test("a workflow with no jobs throws rather than reporting an empty set", () => {
    expect(() => parseJobs("name: CI\non: [push]\n")).toThrow("declares no jobs");
  });
});

describe("the local-safety verdict is fail-closed", () => {
  const jobsOf = (...ids: string[]) =>
    parseJobs(workflow(ids.map((id) => `  ${id}:\n    steps:\n      - run: "true"\n`).join("")));

  test("a classified-safe job carries no refusal", () => {
    expect(jobsOf("repo-gates")[0].refusal).toBeNull();
  });

  test("a job named in neither table is refused, so a new CI job is a decision", () => {
    expect(jobsOf("some-new-job")[0].refusal).toBe(UNCLASSIFIED);
  });

  test("the destructive jobs are refused with the damage named", () => {
    const [rust, ui] = jobsOf("cargo-tests", "ui-check");
    expect(rust.refusal).toContain("tools/cargo-hermetic");
    expect(ui.refusal).toContain("machine.toml");
  });

  // The two tables answer the same question and must not both answer it. A job in both
  // would run here on the strength of RUNS_HERE while REFUSED records why it must not.
  test("no job is in both tables", () => {
    for (const id of Object.keys(RUNS_HERE)) expect(REFUSED[id]).toBeUndefined();
  });
});

describe("this repository's own ci.yml", () => {
  // Read the real file, because the whole point of the tool is that the gate list is not
  // written down twice. Nothing here asserts WHICH gates exist — that would be the second
  // copy again. It asserts that the real workflow parses and that a job this tool offers
  // to run is never an empty one.
  test("every job parses, and a job that runs here has work in it", async () => {
    const text = await Bun.file(`${import.meta.dir}/../.github/workflows/ci.yml`).text();
    const parsed = parseJobs(text);
    expect(parsed.map((j) => j.id)).toContain("repo-gates");
    for (const job of parsed) {
      if (job.refusal === null) expect(job.steps.length).toBeGreaterThan(0);
    }
  });
});
