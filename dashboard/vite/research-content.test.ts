import { describe, expect, test } from "bun:test";
import { groupUpstreams, matches, resolveResearchLink, toEntry } from "../src/lib/research/content";

const link = (path: string) => `/base${path}`;

describe("resolveResearchLink", () => {
  test("keeps a link between entries in the app, anchor and base kept", () => {
    expect(resolveResearchLink("why-sjel.md", link)).toBe("/base/research/why-sjel");
    expect(resolveResearchLink("README.md#ideas", link)).toBe("/base/research#ideas");
  });
  test("sends any other repository file to GitHub", () => {
    expect(resolveResearchLink("../libs/inference/README.md#the-shape", link)).toBe(
      "https://github.com/larsboes/Sjel/blob/main/libs/inference/README.md#the-shape",
    );
  });
  test("leaves external links and in-page anchors alone", () => {
    expect(resolveResearchLink("https://example.org/a", link)).toBe("https://example.org/a");
    expect(resolveResearchLink("#top", link)).toBe("#top");
  });
});

describe("toEntry", () => {
  test("takes the title from the # line and drops it from the body", () => {
    const entry = toEntry("why-sjel.md", "# Why Sjel exists\n\nFirst paragraph.\n");
    expect(entry).toEqual({ slug: "why-sjel", file: "why-sjel.md", title: "Why Sjel exists", body: "First paragraph.\n" });
  });
  test("README is the index", () => {
    expect(toEntry("README.md", "# Research\n").slug).toBe("");
  });
});

describe("groupUpstreams", () => {
  const row = (name: string, verdict: string) => ({ name, verdict, url: "", license: "", summary: "" });
  test("groups by relation and keeps an unknown verdict visible", () => {
    const groups = groupUpstreams([row("b", "adopt"), row("a", "adopt"), row("g", "watch"), row("x", "someday")]);
    expect(groups.find((g) => g.id === "built-on")?.rows.map((r) => r.name)).toEqual(["a", "b"]);
    expect(groups.find((g) => g.id === "watching")?.rows.map((r) => r.name)).toEqual(["g"]);
    expect(groups.find((g) => g.id === "other")?.rows.map((r) => r.name)).toEqual(["x"]);
  });
});

test("matches is case-insensitive and an empty query matches everything", () => {
  expect(matches("", "anything")).toBe(true);
  expect(matches("OLL", "ollama", null)).toBe(true);
  expect(matches("zed", "ollama")).toBe(false);
});

import { percent, suites, verdict } from "../src/lib/research/content";

describe("benchmark suites", () => {
  const run = (model: string, accuracy: number, p50: number, cases: [string, unknown, unknown][]) => ({
    suite: "decisions",
    file: `${model}.json`,
    result: {
      suite: "decisions", date: "2026-09-30", backend: "ollama", model, host: "Mac", n: cases.length,
      correct: 0, accuracy, latency_ms: { p50, p95: p50 },
      cases: cases.map(([id, expected, got]) => ({ id, expected, got, confidence: null, ms: 1 })),
    },
  });
  test("orders runs by accuracy, then by the faster median", () => {
    const [suite] = suites([run("slow", 0.9, 500, []), run("best", 0.95, 900, []), run("fast", 0.9, 60, [])]);
    expect(suite.runs.map((r) => r.result.model)).toEqual(["best", "fast", "slow"]);
  });
  test("scores a case by exact answer, and a missing case as null", () => {
    const r = run("m", 1, 1, [["a", "spam", "spam"], ["b", true, false]]);
    expect(verdict(r, "a")).toBe(true);
    expect(verdict(r, "b")).toBe(false);
    expect(verdict(r, "c")).toBeNull();
  });
  test("percent keeps one decimal", () => {
    expect(percent(0.757)).toBe("75.7%");
  });
});
