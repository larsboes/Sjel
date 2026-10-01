import { describe, expect, test } from "bun:test";

import { type Case, loadCases, percentile, readAnswer, summarize } from "./decision-bench.ts";

const choice: Case = {
  id: "c", task: "t", state: "s", expected: "a",
  question: { type: "choice", instructions: "?", criteria: { a: "A", b: "B" } },
};
const noul: Case = { ...choice, id: "n", expected: true, question: { ...choice.question, type: "noul" } };

describe("decision-bench scoring (no network)", () => {
  test("nearest-rank percentiles, and 0 for no samples", () => {
    const ms = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    expect(percentile(ms, 50)).toBe(50);
    expect(percentile(ms, 95)).toBe(100);
    expect(percentile([], 50)).toBe(0);
  });

  test("reads a choice, a noul at the 0.5 boundary, and an unreadable answer", () => {
    expect(readAnswer(choice, { choice: "b", confidence: 0.4 })).toEqual({ got: "b", confidence: 0.4 });
    expect(readAnswer(noul, { noul: 0.5 })).toEqual({ got: true, confidence: 0.5 });
    expect(readAnswer(noul, { noul: 0.2 })).toEqual({ got: false, confidence: 0.8 });
    expect(readAnswer(choice, { noul: 0.9 })).toEqual({ got: null, confidence: null });
  });

  test("a failed case counts as wrong and is left out of the latency", () => {
    const s = summarize([
      { id: "1", expected: "a", got: "a", confidence: 1, ms: 100 },
      { id: "2", expected: true, got: false, confidence: 1, ms: 300 },
      { id: "3", expected: "a", got: null, confidence: null, ms: 0 },
    ]);
    expect(s).toEqual({ n: 3, correct: 1, accuracy: 0.3333, latency_ms: { p50: 100, p95: 300 } });
  });

  test("every case has exactly one expected answer among its criteria", () => {
    const cases = loadCases();
    expect(new Set(cases.map((c) => c.id)).size).toBe(cases.length);
    for (const c of cases) {
      if (c.question.type === "choice") expect(Object.keys(c.question.criteria)).toContain(c.expected as string);
      else expect(typeof c.expected).toBe("boolean");
    }
  });
});
