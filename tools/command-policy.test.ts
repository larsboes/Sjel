// Command policy (docs/plans/agent.md §3) for pi: every case in
// Packs/security/extensions/command-policy/cases.toml, plus the extension's card behaviour.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { classify, classifyWrite, type Scope } from "../Packs/security/extensions/command-policy/classify.ts";
import commandPolicy from "../Packs/security/extensions/command-policy/index.ts";

const dir = join(import.meta.dir, "../Packs/security/extensions/command-policy");
const table = Bun.TOML.parse(readFileSync(join(dir, "cases.toml"), "utf8")) as {
  case: { command: string; class: string; rule?: string }[];
  write: { path: string; class: string }[];
};
const scope: Scope = { cwd: "/home/u/proj", home: "/home/u", writable: ["/tmp"] };

describe("command policy cases", () => {
  test("the table covers every rule ID the spec defines", () => {
    const pinned = new Set(table.case.map((c) => c.rule).filter(Boolean));
    for (const id of ["B1", "B2", "B3", "B4", "B5", "B6", "B7", "A1", "A2", "A3", "A4", "A5", "A6", "R1", "R2", "R3", "R4"]) {
      expect(`${id}: ${pinned.has(id)}`).toBe(`${id}: true`);
    }
  });

  test("every command case", () => {
    const wrong: string[] = [];
    for (const c of table.case) {
      const v = classify(c.command, scope);
      if (v.class !== c.class || (c.rule && v.rule !== c.rule)) {
        wrong.push(`${JSON.stringify(c.command)}: want ${c.class}${c.rule ? `/${c.rule}` : ""}, got ${v.class}/${v.rule} (${v.why})`);
      }
    }
    expect(wrong).toEqual([]);
  });

  test("every write case", () => {
    const wrong: string[] = [];
    for (const w of table.write) {
      const v = classifyWrite(w.path, scope, "write");
      if (v.class !== w.class) wrong.push(`${w.path}: want ${w.class}, got ${v.class}`);
    }
    expect(wrong).toEqual([]);
  });

  test("the log never carries arguments", () => {
    const v = classify("curl -H 'Authorization: Bearer s3cret' https://example.com", scope);
    expect(JSON.stringify(v.programs)).not.toContain("s3cret");
  });
});

describe("command policy extension", () => {
  type Handler = (event: unknown, ctx: unknown) => Promise<{ block?: boolean; reason?: string } | undefined>;
  const load = () => {
    let handler: Handler | undefined;
    commandPolicy({ on: (name: string, fn: Handler) => { if (name === "tool_call") handler = fn; } } as never);
    return handler!;
  };
  const ctx = (answers: string[]) => ({
    cwd: "/home/u/proj",
    hasUI: true,
    asked: [] as string[],
    ui: { select: async (title: string) => { (ctx as never as { last: string }).last = title; return answers.shift(); } },
  });

  test("allow runs without a card", async () => {
    const h = load();
    const c = ctx([]);
    expect(await h({ toolName: "bash", input: { command: "git status" } }, c)).toBeUndefined();
  });

  test("ask without a UI refuses", async () => {
    const h = load();
    const r = await h({ toolName: "bash", input: { command: "git commit -m x" } }, { cwd: "/home/u/proj", hasUI: false });
    expect(r?.block).toBe(true);
  });

  test("a session grant covers the exact call and nothing else", async () => {
    const h = load();
    expect(await h({ toolName: "bash", input: { command: "git commit -m x" } }, ctx(["Allow this exact call for this session"]))).toBeUndefined();
    expect(await h({ toolName: "bash", input: { command: "git commit -m x" } }, ctx([]))).toBeUndefined();
    const other = await h({ toolName: "bash", input: { command: "git commit -m y" } }, ctx(["Deny"]));
    expect(other?.block).toBe(true);
  });

  test("block defaults to deny and never becomes a session grant", async () => {
    const h = load();
    expect((await h({ toolName: "bash", input: { command: "rm -rf x" } }, ctx(["Deny"])))?.block).toBe(true);
    expect(await h({ toolName: "bash", input: { command: "rm -rf x" } }, ctx(["Allow once"]))).toBeUndefined();
    expect((await h({ toolName: "bash", input: { command: "rm -rf x" } }, ctx([undefined as never])))?.block).toBe(true);
  });

  test("vault_exec commands are classified like bash", async () => {
    const h = load();
    expect((await h({ toolName: "vault_exec", input: { cmd: "rm -rf x", env_file: "a" } }, ctx(["Deny"])))?.block).toBe(true);
  });

  test("other tools pass untouched", async () => {
    const h = load();
    expect(await h({ toolName: "read", input: { path: "/etc/passwd" } }, ctx([]))).toBeUndefined();
  });
});
