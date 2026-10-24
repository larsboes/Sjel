import { describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { command } from "./agent-eval.ts";

describe("offline evaluator command boundary", () => {
  test("review renders each attempt with the case's claims as checkboxes", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agent-eval-review-"));
    const file = join(dir, "run.jsonl");
    writeFileSync(file, JSON.stringify({ started: "t", case_id: "C01", harness: "sjel", repeat: 0, termination: "final", elapsed_ms: 1, model: "m", calls: [{ tool: "read", status: "denied", content: "path is not an allowed input file" }], error: null, answer: "line one\nline two" }) + "\n");
    const { code, output } = await command(["review", file]);
    expect(code).toBe(0);
    expect(output).toContain("- [ ] C01.1: ");
    expect(output).toContain("`read` → denied: path is not an allowed input file");
    expect(output).toContain("> line one\n> line two");
    rmSync(dir, { recursive: true });
  });

  test("replay errors and unsupported execution have nonzero exit codes", async () => {
    const root = mkdtempSync(join(tmpdir(), "sjel-eval-replay-"));
    try {
      const trace = join(root, "requests.json");
      writeFileSync(trace, JSON.stringify({ case_id: "C01", calls: [{ tool: "list", args: { extra: true } }], answer: "" }));
      expect((await command(["replay", trace])).code).toBe(1);
      writeFileSync(trace, JSON.stringify({ case_id: "R01", calls: [{ tool: "sandbox_python", args: { code: "print(1)" } }], answer: "" }));
      expect((await command(["replay", trace])).code).toBe(2);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
  test("rejects oversized files and a FIFO with no writer", async () => {
    const root = mkdtempSync(join(tmpdir(), "sjel-eval-input-"));
    try {
      const large = join(root, "large.json");
      writeFileSync(large, Buffer.alloc(4_000_001));
      await expect(command(["replay", large])).rejects.toThrow("4 MB");
      const fifo = join(root, "pipe");
      execFileSync("mkfifo", [fifo]);
      await expect(command(["replay", fifo])).rejects.toThrow("regular file");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
  test("checks the catalogue without launching a solver", async () => {
    const result = await command(["check"]);
    expect(result.code).toBe(0);
    expect(JSON.parse(result.output)).toMatchObject({ cases: 8, results_status: "not_run" });
  });

  test("exports workspace inputs without answer keys", async () => {
    const result = await command(["input", "R04"]);
    const input = JSON.parse(result.output);
    expect(input.id).toBe("R04");
    expect(input).not.toHaveProperty("oracle");
    expect(input).not.toHaveProperty("provenance");
    expect(Object.hasOwn(input.files, "hypotheses.md")).toBe(true);
    expect(input.policy.probe_max_attempts).toBe(1);
  });

  test("runs live only from a complete manifest, and rejects extra arguments and unknown cases", async () => {
    await expect(command(["run"])).rejects.toThrow("Usage");
    await expect(command(["run", "pi", "C01"])).rejects.toThrow("Usage");
    const dir = mkdtempSync(join(tmpdir(), "agent-eval-cli-"));
    const manifest = join(dir, "manifest.json");
    writeFileSync(manifest, JSON.stringify({ cases: ["C01"], harnesses: ["pi"] }));
    await expect(command(["run", manifest])).rejects.toThrow("manifest repeats");
    rmSync(dir, { recursive: true });
    await expect(command(["check", "--run"])).rejects.toThrow("Usage");
    await expect(command(["input", "missing"])).rejects.toThrow("unknown case");
    await expect(command(["grade", "R04", "unused"])).rejects.toThrow("C02 only");
  });
});
