import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { Broker, validateSuite, type EvalCase } from "./lib/agent-eval.ts";
import { edges, sections } from "./lib/agent-eval-retrieval.ts";

const files = {
  "a.md": "# A\nintro\n## A.1\nsee b.md and [grafeo]\n### A.1.1\ndeep\n## A.2\nlast",
  "b.md": "plain text, no headings",
  "u.toml": "# register\n[grafeo]\nverdict = \"reject\"\n[libsql]\nverdict = \"adopt\"",
};

test("sections split Markdown at headings and TOML at table rows, keeping 1-based lines", () => {
  expect(sections(files).map((s) => `${s.path}:${s.line} ${s.heading}`)).toEqual([
    "a.md:1 # A", "a.md:3 ## A.1", "a.md:5 ### A.1.1", "a.md:7 ## A.2",
    "b.md:1 b.md",
    "u.toml:1 u.toml", "u.toml:2 [grafeo]", "u.toml:4 [libsql]",
  ]);
});

test("edges link headings to the sections under them, and sections to the files and rows they name", () => {
  const all = sections(files);
  const at = (path: string, line: number) => all.find((s) => s.path === path && s.line === line)!.id;
  const got = edges(all).map((e) => `${e.kind} ${all[e.a].path}:${all[e.a].line} ${all[e.b].path}:${all[e.b].line}`).sort();
  expect(got).toEqual([
    "contains a.md:1 a.md:3",
    "contains a.md:1 a.md:7",
    "contains a.md:3 a.md:5",
    `references a.md:3 b.md:1`,
    `references a.md:3 u.toml:2`,
  ].sort());
  expect(at("a.md", 3)).toBe(1);
});

test("a recorded extra tool call shares the transcript and the call budget", () => {
  const suite = validateSuite(JSON.parse(readFileSync(new URL("../research/benchmarks/agent-harness/cases.json", import.meta.url), "utf8")));
  const broker = new Broker(suite.cases.find((c) => c.id === "D01") as EvalCase);
  broker.record("retrieve", "ok", "[]");
  broker.call("list", {});
  const report = broker.report("", "live_harness");
  expect(report.calls.map((c) => `${c.index} ${c.tool} ${c.status}`)).toEqual(["0 retrieve ok", "1 list ok"]);
  expect(report.replay_status).toBe("completed");
});
