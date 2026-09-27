// tools/lib/pi-agent-file.test.ts — the translation from a Claude-Code-native agent
// file to the shape pi-subagents loads.
//
// The load-bearing cases are the ones that assert a FAILURE rather than an output:
// a tool name with no pi equivalent, and a file with no `tools:` line at all. Both
// would otherwise produce an agent that loads, answers, and has no read tool —
// pi-subagents reports the first as `tools-error` and drops it from the allowlist
// (upstream issue #75), and defaults the second to all seven builtins including
// write and edit. Neither failure announces itself, so both are tests.
//
// The real Packs are read from disk rather than mocked, so adding an agent file
// with a tool this map does not know is caught here instead of in a run.
//
// Run: bun test tools/lib/pi-agent-file.test.ts

import { describe, expect, test } from "bun:test";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { PI_BUILTIN_TOOL_NAMES, PI_TOOL_NAME, translateAgentForPi } from "./pi-agent-file.ts";

const SJEL_ROOT = resolve(import.meta.dir, "..", "..");

/** Every agent file the Packs carry, as [label, content]. */
function packAgentFiles(): [string, string][] {
  const packs = join(SJEL_ROOT, "Packs");
  const found: [string, string][] = [];
  for (const pack of readdirSync(packs).sort()) {
    const dir = join(packs, pack, "agents");
    if (!existsSync(dir)) continue;
    for (const file of readdirSync(dir).sort()) {
      if (!file.endsWith(".md")) continue;
      const label = `Packs/${pack}/agents/${file}`;
      found.push([label, readFileSync(join(dir, file), "utf8")]);
    }
  }
  return found;
}

/** Split the way pi's own parseFrontmatter does, so the test measures the real shape. */
function split(content: string): { frontmatter: string[]; body: string } {
  const text = content.startsWith("\uFEFF") ? content.slice(1) : content;
  const end = text.indexOf("\n---", 3);
  return {
    frontmatter: text.slice(4, end).split("\n"),
    body: text.slice(end + 4).trim(),
  };
}

function field(frontmatter: string[], key: string): string | undefined {
  const line = frontmatter.find((candidate) => candidate.startsWith(`${key}:`));
  return line?.slice(key.length + 1).trim();
}

describe("translateAgentForPi — the real Packs", () => {
  const files = packAgentFiles();

  test("there are agent files to translate", () => {
    expect(files.length).toBeGreaterThan(0);
  });

  for (const [label, content] of files) {
    test(`${label} translates to a loadable pi agent`, () => {
      const translated = translateAgentForPi(content, label);
      const { frontmatter, body } = split(translated);

      // Identity survives: pi dispatches on `name:`, and the description is what
      // the Agent tool's listing shows.
      expect(field(frontmatter, "name")).toBe(field(split(content).frontmatter, "name"));
      expect(field(frontmatter, "description")).toBe(field(split(content).frontmatter, "description"));

      // The pin is gone (decision of 2026-09-16: pi inherits the session model).
      expect(frontmatter.some((line) => line.startsWith("model:"))).toBe(false);

      // Every tool is a pi builtin — the assertion that would have caught a
      // passthrough of `Read`/`Grep`/`Glob`.
      const tools = field(frontmatter, "tools")?.split(",").map((name) => name.trim()) ?? [];
      expect(tools.length).toBeGreaterThan(0);
      for (const tool of tools) expect(PI_BUILTIN_TOOL_NAMES).toContain(tool);

      // The body is the system prompt: byte-for-byte, or the agent's contract changed.
      expect(body).toBe(split(content).body);
    });
  }

  test("every Claude Code name in use is mapped", () => {
    const seen = new Set<string>();
    for (const [, content] of files) {
      const tools = field(split(content).frontmatter, "tools") ?? "";
      for (const name of tools.split(",").map((candidate) => candidate.trim()).filter(Boolean)) {
        seen.add(name);
      }
    }
    expect(seen.size).toBeGreaterThan(0);
    for (const name of seen) expect(Object.keys(PI_TOOL_NAME)).toContain(name);
  });
});

describe("translateAgentForPi — the two names that differ in kind", () => {
  test("Glob becomes find, which is pi's search tool", () => {
    const translated = translateAgentForPi("---\ntools: Read, Grep, Glob\n---\n\nbody\n", "t.md");
    expect(field(split(translated).frontmatter, "tools")).toBe("read, grep, find");
  });

  test("Bash becomes bash — the bibliography auditor genuinely needs a shell", () => {
    const translated = translateAgentForPi("---\ntools: Read, Grep, Glob, Bash\n---\n\nbody\n", "t.md");
    expect(field(split(translated).frontmatter, "tools")).toBe("read, grep, find, bash");
  });
});

describe("translateAgentForPi — refusals", () => {
  test("an unmapped tool name throws instead of passing through", () => {
    expect(() => translateAgentForPi("---\ntools: Read, NotebookEdit\n---\n\nbody\n", "t.md"))
      .toThrow(/no pi equivalent/);
  });

  test("a missing tools: line throws, because pi would grant all seven builtins", () => {
    expect(() => translateAgentForPi("---\nname: x\nmodel: sonnet\n---\n\nbody\n", "t.md"))
      .toThrow(/write and edit access/);
  });

  test("an empty tools: list throws", () => {
    expect(() => translateAgentForPi("---\ntools:\n---\n\nbody\n", "t.md")).toThrow(/present but empty/);
  });

  test("a file with no frontmatter throws", () => {
    expect(() => translateAgentForPi("# just a body\n", "t.md")).toThrow(/no frontmatter block/);
  });

  test("an unterminated frontmatter block throws", () => {
    expect(() => translateAgentForPi("---\ntools: Read\n", "t.md")).toThrow(/unterminated/);
  });
});

describe("translateAgentForPi — provenance", () => {
  test("the generated copy says where it came from and why the pin is gone", () => {
    const translated = translateAgentForPi(
      "---\nname: council-clerk\ntools: Read, Grep, Glob\nmodel: sonnet\n---\n\nbody\n",
      "Packs/deliberation/agents/council-clerk.md",
    );
    expect(translated).toContain("Packs/deliberation/agents/council-clerk.md");
    expect(translated).toContain("do not edit this copy");
    expect(translated).toContain("inherits the session model");
  });

  test("a file that carried no pin says so rather than implying one was dropped", () => {
    const translated = translateAgentForPi("---\nname: x\ntools: Read\n---\n\nbody\n", "t.md");
    expect(translated).toContain("carried no 'model:' pin");
  });
});
