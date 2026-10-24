import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { validateSuite } from "./lib/agent-eval.ts";
import { compareProjection, evaluateProjection, projectionInput, sandboxCommand, sandboxProbe } from "./lib/agent-eval-sandbox.ts";

const suite = validateSuite(JSON.parse(readFileSync(join(import.meta.dir, "../research/benchmarks/agent-harness/cases.json"), "utf8")));
const fixture = suite.cases.find((c) => c.id === "C02")!;

describe("projection sandbox parent/guest boundary (no containers)", () => {
  test("the guest receives changes, never expected rows or other oracle fields", () => {
    const input = JSON.parse(projectionInput(fixture, "class Projection: pass"));
    expect(Object.keys(input).sort()).toEqual(["sequences", "source"]);
    expect(JSON.stringify(input)).not.toContain("expected_rows");
    expect(JSON.stringify(input)).not.toContain("critical_failures");
    expect(input.sequences[0][0]).toEqual({ id: "a", revision: 2, op: "upsert", value: "new" });
  });

  test("source size and wrong-case execution are rejected before any process", () => {
    expect(() => projectionInput({ ...fixture, id: "R01" }, "pass")).toThrow("C02 only");
    expect(() => projectionInput(fixture, "x".repeat(128_001))).toThrow("limit");
    expect(() => projectionInput({ ...fixture, oracle: { ...fixture.oracle, event_sequences: [] } }, "pass")).toThrow("sequences");
    const poisoned = structuredClone(fixture);
    const sequences = poisoned.oracle.event_sequences as { steps: { change: Record<string, unknown> }[] }[];
    sequences[0]!.steps[0]!.change.expected_rows = { stolen: "answer" };
    expect(() => projectionInput(poisoned, "pass")).toThrow("change shape");
  });

  test("parent checks every event, including stale deletes", () => {
    const sequences = fixture.oracle.event_sequences as { steps: { expected_rows: Record<string, unknown> }[] }[];
    const snapshots = sequences.map((s) => s.steps.map((step) => ({ ...step.expected_rows })));
    expect(compareProjection(fixture, JSON.stringify({ snapshots })).status).toBe("passed");
    snapshots.at(-1)![1] = {};
    expect(compareProjection(fixture, JSON.stringify({ snapshots }))).toMatchObject({ status: "failed", failures: [{ sequence: sequences.length - 1, step: 1 }] });
  });

  test("guest self-scores, exceptions and malformed output cannot pass", () => {
    expect(compareProjection(fixture, '{"status":"passed"}').status).toBe("failed");
    expect(compareProjection(fixture, '{"error":"RuntimeError"}').status).toBe("failed");
    expect(compareProjection(fixture, 'noise\n{"snapshots":[]}').status).toBe("failed");
    expect(compareProjection(fixture, 'x'.repeat(128_001)).status).toBe("failed");
  });

  test("command cannot accept mutable image tags, arbitrary code or container names", () => {
    expect(() => sandboxCommand("python:3.13-slim", "sjel-agent-eval-00000000-0000-0000-0000-000000000000", "probe")).toThrow("immutable");
    expect(() => sandboxCommand(`sha256:${"a".repeat(64)}`, "other-container", "probe")).toThrow("name");
    expect(() => sandboxCommand(`sha256:${"a".repeat(64)}`, "sjel-agent-eval-00000000-0000-0000-0000-000000000000", "pass" as "probe")).toThrow("entrypoint");
  });

  test("both entrypoints use the same bounded no-network policy", () => {
    for (const mode of ["probe", "projection"] as const) {
      const args = sandboxCommand(`sha256:${"a".repeat(64)}`, "sjel-agent-eval-00000000-0000-0000-0000-000000000000", mode);
      expect(args).toContain("--pull=never");
      expect(args.slice(args.indexOf("--network"), args.indexOf("--network") + 2)).toEqual(["--network", "none"]);
      expect(args).toContain("--read-only");
      expect(args).toContain("65534:65534");
      expect(args).toContain("no-new-privileges");
      expect(args).toContain("--pids-limit");
      expect(args.some((a) => ["--mount", "--volume", "-v", "--env", "-e", "--privileged", "--publish", "-p"].includes(a))).toBe(false);
    }
  });
});

describe.skipIf(process.env.SJEL_AGENT_EVAL_SANDBOX_INTEGRATION !== "1")("OrbStack sandbox integration", () => {
  const conforming = `class Projection:
    def __init__(self):
        self.rows = {}
        self.revisions = {}
    def apply(self, change):
        key = change["id"]
        revision = change["revision"]
        if revision <= self.revisions.get(key, 0):
            return
        self.revisions[key] = revision
        if change["op"] == "delete":
            self.rows.pop(key, None)
        else:
            self.rows[key] = change["value"]
`;

  test("the active local sandbox establishes all required checks", async () => {
    expect(await sandboxProbe()).toMatchObject({ supported: true });
  });

  test("the original fixture fails and a contract-conforming candidate passes", async () => {
    expect(await evaluateProjection(fixture, fixture.input.files["projection.py"]!.content)).toMatchObject({ status: "failed", transitions: 18 });
    expect(await evaluateProjection(fixture, conforming)).toMatchObject({ status: "passed", transitions: 18, failures: [] });
  });

  test("candidate stack inspection cannot recover the parent oracle", async () => {
    const inspect = `import inspect
for frame in inspect.stack():
    assert "expected_rows" not in frame.frame.f_locals
    assert "oracle" not in frame.frame.f_locals
`;
    expect(await evaluateProjection(fixture, inspect + conforming)).toMatchObject({ status: "passed" });
  });

  test("stdout spoofing and oversized output fail closed", async () => {
    expect(await evaluateProjection(fixture, `print('{"status":"passed"}')\n` + conforming)).toMatchObject({ status: "failed" });
    expect(await evaluateProjection(fixture, `print("x" * 150000)\n` + conforming)).toMatchObject({ status: "failed", reason: "sandbox output limit exceeded" });
  });

  test("a nonterminating candidate is killed at the deadline", async () => {
    expect(await evaluateProjection(fixture, "while True: pass")).toMatchObject({ status: "failed", reason: "sandbox deadline exceeded" });
  }, 15_000);
});
