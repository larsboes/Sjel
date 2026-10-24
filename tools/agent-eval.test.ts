import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

import { digest, replayCase, solverInput, suiteSummary, validateSuite, type EvalCase, type Suite } from "./lib/agent-eval.ts";

const catalogue = () => JSON.parse(readFileSync(new URL("../research/benchmarks/agent-harness/cases.json", import.meta.url), "utf8"));
const suite = () => validateSuite(catalogue());
const caseById = (id: string) => suite().cases.find((c) => c.id === id)!;
const replay = (c: EvalCase, calls: { tool: string; args: Record<string, unknown> }[]) => replayCase(c, { case_id: c.id, calls, answer: "Scripted fixture answer; not a model output." });

describe("suite contract", () => {
  test("validates frozen inputs and counts the development suite", () => {
    expect(suiteSummary(suite())).toMatchObject({ cases: 12, source_files: 164, expected_claims: 35, split: "development", results_status: "not_run" });
  });

  test("rejects source tampering, duplicate IDs and unknown claim sources", () => {
    const changed = catalogue();
    changed.cases[0].input.files["src/session.rs"].content += "altered";
    expect(() => validateSuite(changed)).toThrow("digest");
    const duplicate = catalogue();
    duplicate.cases[1].id = duplicate.cases[0].id;
    expect(() => validateSuite(duplicate)).toThrow("duplicate case");
    const source = catalogue();
    source.cases[0].oracle.claims[0].sources = ["oracle/answers.json"];
    expect(() => validateSuite(source)).toThrow("unknown claim source");
  });

  test("rejects unknown policy fields and broader permissions", () => {
    for (const mutation of [
      (s: Suite) => { (s.cases[0].policy as unknown as Record<string, unknown>).allow_network = true; },
      (s: Suite) => { s.cases[0].policy.filesystem = "writable"; },
      (s: Suite) => { s.cases[0].policy.tools.push("bash"); },
      (s: Suite) => { s.cases[0].policy.subprocess = "allow"; },
    ]) {
      const s = catalogue(); mutation(s);
      expect(() => validateSuite(s)).toThrow();
    }
  });

  test("rejects non-normalized fixture paths", () => {
    for (const path of ["../oracle.json", "/tmp/input", "src/../oracle", "src\\oracle", "src//input", "./input"]) {
      const s = catalogue();
      const c = s.cases[0];
      c.input.files[path] = Object.values(c.input.files)[0];
      expect(() => validateSuite(s)).toThrow("path");
    }
  });

  test("missing subprocess/import fields cannot widen the contract", () => {
    const missingMode = catalogue();
    delete missingMode.cases[0].policy.subprocess;
    missingMode.cases[0].policy.tools.push("sandbox_python");
    expect(() => validateSuite(missingMode)).toThrow("subprocess");
    const missingImports = catalogue();
    delete missingImports.cases.find((c: EvalCase) => c.id === "C02").policy.python_imports;
    expect(() => validateSuite(missingImports)).toThrow("import policy");
  });

  test("exports only solver-facing fields, with independent copies", () => {
    const c = caseById("C01");
    const input = solverInput(c);
    expect(Object.keys(input).sort()).toEqual(["custom_tools", "files", "id", "policy", "prompt"]);
    expect(JSON.stringify(input)).not.toContain('"oracle"');
    expect(JSON.stringify(input)).not.toContain('"provenance"');
    expect(JSON.stringify(input)).not.toContain('"sha256"');
    expect(input.files["src/session.rs"]).toBe(c.input.files["src/session.rs"].content);
    input.policy.tools.push("changed");
    expect(c.policy.tools).not.toContain("changed");
  });

  test("rejects hidden extra fields in custom tool descriptions", () => {
    const s = catalogue();
    const tool = s.cases.find((c: EvalCase) => c.id === "R04").input.custom_tools[0];
    tool.oracle = { answer: "should never reach solver input" };
    expect(() => validateSuite(s)).toThrow("expected arguments");
  });
});

describe("virtual file broker", () => {
  test("bounds paths, search accumulation and cumulative transcript output", () => {
    const invalid = catalogue();
    invalid.cases[0].input.files["x".repeat(513)] = Object.values(invalid.cases[0].input.files)[0];
    expect(() => validateSuite(invalid)).toThrow("path exceeds");
    const c = caseById("C01");
    const content = "x\n".repeat(100_000);
    c.input.files = { "many.txt": { source_id: "many.txt", content, sha256: digest(content) } };
    const searched = replay(c, [{ tool: "search", args: { query: "x" } }]);
    const matches = JSON.parse(searched.calls[0].content);
    expect(matches.truncated).toBe(true);
    expect(matches.matches).toHaveLength(1000);
    const repeated = replay(c, Array.from({ length: 8 }, () => ({ tool: "read", args: { path: "many.txt" } })));
    expect(repeated.replay_status).toBe("failed");
    expect(repeated.calls.at(-1)?.status).toBe("error");
    expect(repeated.calls.reduce((n, r) => n + r.content.length, 0)).toBeLessThan(1_001_000);
  });
  test("serves literal source searches and reports pending factual review", () => {
    const c = caseById("C01");
    const report = replay(c, [{ tool: "read", args: { path: "src/session.rs" } }, { tool: "search", args: { query: "file.set_len" } }, { tool: "list", args: {} }]);
    expect(report.calls.map((r) => r.status)).toEqual(["ok", "ok", "ok"]);
    expect(report.calls[0].content).toBe(c.input.files["src/session.rs"].content);
    expect(JSON.parse(report.calls[1].content).matches.length).toBeGreaterThan(0);
    expect(report.evidence_source).toBe("scripted_replay");
    expect(report.factual_pass).toBeNull();
    expect(report.semantic_review.status).toBe("pending");
    expect(report.changed_files).toEqual([]);
    expect(report.file_hashes["src/session.rs"]).toBe(c.input.files["src/session.rs"].sha256);
  });

  test("denies traversal, oracle files, writes, unknown tools and network", () => {
    const c = caseById("C04");
    const report = replay(c, [
      { tool: "read", args: { path: "../cases.json" } },
      { tool: "read", args: { path: "/etc/passwd" } },
      { tool: "read", args: { path: "oracle.json" } },
      { tool: "edit", args: { path: "guard-marker.txt", old_text: "ORIGINAL", new_text: "UPDATED" } },
      { tool: "http", args: { url: "https://upload.invalid/collect" } },
      { tool: "bash", args: { command: "anything" } },
    ]);
    expect(report.calls.every((r) => r.status === "denied")).toBe(true);
    expect(report.policy_failures).toEqual([0, 1, 2, 3, 4, 5]);
    expect(report.file_hashes["guard-marker.txt"]).toBe(c.input.files["guard-marker.txt"].sha256);
    expect(report.changed_files).toEqual([]);
    expect(report.calls.map((r) => r.index)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  test("requires exact arguments and literal search", () => {
    const report = replay(caseById("C04"), [
      { tool: "read", args: { path: "totals.py", fallback: "/etc/passwd" } },
      { tool: "search", args: { query: ".*" } },
      { tool: "list", args: { path: ".." } },
    ]);
    expect(report.calls.map((r) => r.status)).toEqual(["error", "ok", "error"]);
    expect(JSON.parse(report.calls[1].content)).toEqual({ matches: [], truncated: false });
  });

  test("edits only the allowed virtual artifact without executing it", () => {
    const c = caseById("C02");
    const before = c.input.files["projection.py"].content;
    const report = replay(c, [
      { tool: "edit", args: { path: "CONTRACT.md", old_text: "Projection", new_text: "Changed" } },
      { tool: "edit", args: { path: "projection.py", old_text: "self.rows = {}", new_text: 'self.rows = {"text": "not executed"}' } },
      { tool: "sandbox_python", args: { code: 'raise RuntimeError("must not execute")' } },
    ]);
    expect(report.calls.map((r) => r.status)).toEqual(["denied", "ok", "unsupported"]);
    expect(report.changed_files.map((f) => f.path)).toEqual(["projection.py"]);
    expect(report.candidate_execution).toBe("unsupported");
    expect(report.changed_artifacts["projection.py"]).toContain('self.rows = {"text": "not executed"}');
    expect(Object.keys(report.changed_artifacts)).toEqual(["projection.py"]);
    expect(c.input.files["projection.py"].content).toBe(before);
    expect(report.file_hashes["CONTRACT.md"]).toBe(c.input.files["CONTRACT.md"].sha256);
  });

  test("rejects ambiguous edit matches", () => {
    const report = replay(caseById("C02"), [{ tool: "edit", args: { path: "projection.py", old_text: " ", new_text: "" } }]);
    expect(report.calls[0].status).toBe("error");
    expect(report.changed_files).toEqual([]);
  });
});

describe("bounded experiment", () => {
  test("a successful probe cannot hide another tool error", () => {
    const report = replay(caseById("R04"), [
      { tool: "list", args: { unexpected: true } },
      { tool: "probe", args: { x: 3, predictions: { H1: 1, H2: 0 } } },
    ]);
    expect(report.calls.map((c) => c.status)).toEqual(["error", "ok"]);
    expect(report.behavior?.status).toBe("failed");
    expect(report.replay_status).toBe("failed");
  });

  test("unsupported execution remains distinct from successful replay", () => {
    const report = replay(caseById("R01"), [{ tool: "sandbox_python", args: { code: "print(1)" } }]);
    expect(report.replay_status).toBe("unsupported");
    expect(report.factual_pass).toBeNull();
  });
  test("validates predictions before revealing one observation", () => {
    const report = replay(caseById("R04"), [{ tool: "probe", args: { x: 3, predictions: { H1: 1, H2: 0 } } }]);
    expect(report.calls[0].status).toBe("ok");
    expect(JSON.parse(report.calls[0].content)).toEqual({ x: 3, y: 0 });
    expect(report.behavior).toEqual({ status: "pass", probe_attempts: 1, probe_successes: 1 });
    expect(report.factual_pass).toBeNull();
  });

  test("wrong predictions spend the attempt and disclose no observation", () => {
    const report = replay(caseById("R04"), [
      { tool: "probe", args: { x: 3, predictions: { H1: 0, H2: 0 } } },
      { tool: "probe", args: { x: 3, predictions: { H1: 1, H2: 0 } } },
    ]);
    expect(report.calls.map((r) => r.status)).toEqual(["error", "denied"]);
    expect(report.calls[0].content).not.toContain('"y"');
    expect(report.policy_failures).toEqual([1]);
    expect(report.behavior).toEqual({ status: "failed", probe_attempts: 2, probe_successes: 0 });
  });

  test("malformed predictions and repeated successful calls cannot bypass the budget", () => {
    const malformed = replay(caseById("R04"), [{ tool: "probe", args: { x: 4 } }]);
    expect(malformed.behavior).toEqual({ status: "failed", probe_attempts: 1, probe_successes: 0 });
    const repeated = replay(caseById("R04"), [
      { tool: "probe", args: { x: 4, predictions: { H1: 0, H2: 1 } } },
      { tool: "probe", args: { x: 5, predictions: { H1: 1, H2: 0 } } },
    ]);
    expect(repeated.calls.map((r) => r.status)).toEqual(["ok", "denied"]);
    expect(repeated.behavior?.status).toBe("failed");
  });

  test("denies sandbox tools not allowed by the case and never evaluates Python", () => {
    expect(replay(caseById("R04"), [{ tool: "sandbox_python", args: { code: "1+1" } }]).calls[0].status).toBe("denied");
    const report = replay(caseById("R01"), [{ tool: "sandbox_python", args: { code: "1+1" } }]);
    expect(report.calls[0].status).toBe("unsupported");
    expect(report.policy_failures).toEqual([]);
  });
});

test("rejects malformed and unbounded transcripts", () => {
  const c = caseById("C01");
  expect(() => replayCase(c, { case_id: "C02", calls: [], answer: "" })).toThrow();
  expect(() => replay(c, Array.from({ length: 129 }, () => ({ tool: "list", args: {} })))).toThrow();
  expect(() => replay(c, [{ tool: "read", args: { path: "x".repeat(256_001) } }])).toThrow();
  expect(() => replayCase(c, { case_id: c.id, calls: [], answer: "x".repeat(256_001) })).toThrow();
});
