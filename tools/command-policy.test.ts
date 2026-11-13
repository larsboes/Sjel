// Command policy (docs/plans/agent.md §3) for pi: every case in
// Packs/security/extensions/command-policy/cases.toml, plus the extension's card behaviour.
import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { classify, classifyWrite, resolvePath, type Scope } from "../Packs/security/extensions/command-policy/classify.ts";
import commandPolicy, { APPROVAL_TIMEOUT_MS } from "../Packs/security/extensions/command-policy/index.ts";
import { scopedCommand, scopedWrite } from "../Packs/security/extensions/command-policy/scoped.ts";
import { heavyCommand, powerDeferral } from "../Packs/security/extensions/command-policy/power.ts";

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
    commandPolicy({ on: (name: string, fn: Handler) => { if (name === "tool_call") handler = fn; }, registerCommand: () => {} } as never);
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

  test("routine local work and diagnostics need no UI", async () => {
    const h = load();
    const c = { cwd: "/home/u/proj", hasUI: false };
    for (const command of ["git commit -m x", "rm -f notes.txt", "docker system df", "diskutil list", "launchctl list"]) {
      expect(await h({ toolName: "bash", input: { command } }, c)).toBeUndefined();
    }
  });

  test("ask without a UI refuses", async () => {
    const h = load();
    const r = await h({ toolName: "bash", input: { command: "git push origin main" } }, { cwd: "/home/u/proj", hasUI: false });
    expect(r?.block).toBe(true);
  });

  test("a session grant covers the exact call and nothing else", async () => {
    const h = load();
    expect(await h({ toolName: "bash", input: { command: "git push origin main" } }, ctx(["Allow this exact call for this session"]))).toBeUndefined();
    expect(await h({ toolName: "bash", input: { command: "git push origin main" } }, ctx([]))).toBeUndefined();
    const other = await h({ toolName: "bash", input: { command: "git push origin topic" } }, ctx(["Deny"]));
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

describe("scoped automatic file writes", () => {
  const roots = [resolvePath("/home/u/config-task", scope)];
  test("only named routine file operations qualify", () => {
    for (const command of [
      'cp -pn "$HOME/config-task/source" "$HOME/config-task/source.bak"',
      'cp -n "${HOME}/config-task/source" "${HOME}/config-task/source.bak"',
      "mkdir -p /home/u/config-task/new", "touch /home/u/config-task/new.txt",
    ]) expect(scopedCommand(command, scope, roots)).toBe(true);
    for (const command of [
      "cp a /home/u/config-task/a", "cp -rf a /home/u/config-task/a", "rm -f /home/u/config-task/a",
      "cp -n a /home/u/other/a", "cp -n a /home/u/config-task-sibling/a",
      'cp -n a "$DEST"', 'HOME=/elsewhere cp -n a "$HOME/config-task/a"',
      'cp -n a "$HOME/config-task/a"; git push', 'cp -n a "$(echo /home/u/config-task/a)"',
      "mv a /home/u/config-task/a", "env cp -n a /home/u/config-task/a",
      "cp -n a /home/u/config-task/*.bak", "touch /home/u/config-task/settings.json",
      "touch /home/u/config-task/extensions/policy.ts", "touch /home/u/config-task/.env",
    ]) expect(`${command}: ${scopedCommand(command, scope, roots)}`).toBe(`${command}: false`);
  });
  test("the reported Codex backup qualifies only under an explicit file scope", () => {
    const command = 'cp -pn "$HOME/.codex/config.toml" "$HOME/.codex/config.toml.before-interceptor-escalation.bak"';
    expect(classify(command, scope).class).toBe("ask");
    expect(scopedCommand(command, scope, [])).toBe(false);
    expect(scopedCommand(command, scope, [resolvePath("/home/u/.codex", scope)])).toBe(true);
    expect(scopedCommand('cp -pn a "$HOME/.codex/config.toml"', scope, [resolvePath("/home/u/.codex", scope)])).toBe(false);
  });
  test("direct edits preserve sensitive-target gates and path containment", () => {
    expect(scopedWrite("/home/u/config-task/notes.txt", scope, roots)).toBe(true);
    for (const path of ["/home/u/config-task/../outside", "/home/u/config-task/config.toml", "/home/u/config-task/.ssh/key", "/home/u/config-task/auth.json"]) {
      expect(scopedWrite(path, scope, roots)).toBe(false);
    }
  });
});

describe("scoped path canonicalisation", () => {
  test("symlinks cannot extend a pinned root and cp directory destinations stay gated", () => {
    const root = mkdtempSync(join(tmpdir(), "policy-links-"));
    const task = join(root, "task");
    const outside = join(root, "outside");
    mkdirSync(task); mkdirSync(outside);
    const s = { cwd: root, home: root, writable: [] };
    const roots = [resolvePath(task, s)];
    try {
      symlinkSync(outside, join(task, "escape"));
      expect(scopedWrite(join(task, "escape", "a"), s, roots)).toBe(false);
      expect(scopedCommand(`cp -n source '${task}'`, s, roots)).toBe(false);
      renameSync(task, join(root, "old-task"));
      symlinkSync(outside, task);
      expect(scopedWrite(join(task, "a"), s, roots)).toBe(false);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

describe("permission deferral and scope lifecycle", () => {
  function setup() {
    const handlers = new Map<string, (...args: any[]) => any>();
    const commands = new Map<string, { handler: (...args: any[]) => any }>();
    const messages: unknown[] = [];
    commandPolicy({
      on: (name: string, fn: (...args: any[]) => any) => handlers.set(name, fn),
      registerCommand: (name: string, command: any) => commands.set(name, command),
      sendMessage: (message: unknown) => messages.push(message),
    } as never);
    const choices: (string | undefined)[] = [];
    const prompts: { options: string[]; timeout: number }[] = [];
    const notifications: string[] = [];
    const c = { cwd: homedir(), hasUI: true, ui: {
      select: async (_title: string, options: string[], opts: { timeout: number }) => {
        prompts.push({ options, timeout: opts.timeout });
        return choices.shift();
      },
      setStatus: () => {}, notify: (text: string) => notifications.push(text),
    } };
    return { call: handlers.get("tool_call")!, start: handlers.get("session_start")!, command: commands.get("permissions")!.handler, choices, prompts, notifications, messages, c };
  }
  test("unanswered or explicitly deferred calls do not execute or re-prompt", async () => {
    const h = setup();
    const event = { toolName: "bash", input: { command: "git push origin main" } };
    const r = await h.call(event, h.c);
    expect(r.block).toBe(true);
    expect(r.reason).toContain("NOT denied or approved");
    expect(r.reason).toContain("independent work");
    expect(h.prompts[0].timeout).toBe(APPROVAL_TIMEOUT_MS);
    await h.call(event, h.c);
    expect(h.prompts).toHaveLength(1);
    h.choices.push("1. A2: git push origin main", "Allow next exact call once");
    await h.command("pending", h.c);
    expect(h.messages).toHaveLength(1);
    expect(await h.call(event, h.c)).toBeUndefined();
    expect((await h.call(event, h.c)).block).toBe(true);
  });
  test("an abort grants nothing and does not invent a pending decision", async () => {
    const h = setup();
    const controller = new AbortController();
    controller.abort();
    h.choices.push("Allow this exact call for this session");
    const event = { toolName: "bash", input: { command: "git push" } };
    expect((await h.call(event, { ...h.c, signal: controller.signal })).reason).toContain("aborted");
    await h.command("pending", h.c);
    expect(h.notifications.at(-1)).toContain("No deferred");
    expect((await h.call(event, h.c)).block).toBe(true);
  });
  test("pending approval cannot grant after an abort or session transition", async () => {
    for (const mode of ["abort", "session"]) {
      const h = setup();
      const event = { toolName: "bash", input: { command: "git push" } };
      await h.call(event, h.c);
      const controller = new AbortController();
      const c = { ...h.c, signal: controller.signal, ui: { ...h.c.ui,
        select: async (_title: string, options: string[]) => {
          if (options.includes("Allow next exact call once")) {
            if (mode === "abort") controller.abort();
            else h.start({}, h.c);
            return "Allow next exact call once";
          }
          return options[0];
        },
      } };
      await h.command("pending", c);
      expect(h.messages).toHaveLength(0);
      expect((await h.call(event, h.c)).block).toBe(true);
    }
  });
  test("an outstanding old-session dialog cannot repopulate grants", async () => {
    const h = setup();
    let resolve!: (choice: string) => void;
    const event = { toolName: "bash", input: { command: "git push" } };
    const call = h.call(event, { ...h.c, ui: { ...h.c.ui, select: () => new Promise<string>((done) => { resolve = done; }) } });
    h.start({}, h.c);
    resolve("Allow this exact call for this session");
    expect((await call).block).toBe(true);
    expect((await h.call(event, h.c)).block).toBe(true);
  });
  test("off invalidates outstanding normal and pending approvals", async () => {
    const h = setup();
    let resolve!: (choice: string) => void;
    const event = { toolName: "bash", input: { command: "git push" } };
    const call = h.call(event, { ...h.c, ui: { ...h.c.ui, select: () => new Promise<string>((done) => { resolve = done; }) } });
    resolve("Allow this exact call for this session");
    await h.command("off", h.c);
    expect((await call).block).toBe(true);
    await h.call(event, h.c);
    const c = { ...h.c, ui: { ...h.c.ui, select: async (_title: string, options: string[]) => {
      if (options.includes("Allow next exact call once")) {
        await h.command("off", h.c);
        return "Allow next exact call once";
      }
      return options[0];
    } } };
    await h.command("pending", c);
    expect(h.messages).toHaveLength(0);
    expect((await h.call(event, h.c)).block).toBe(true);
  });
  test("an exact deferred retry is also bound to its working directory", async () => {
    const h = setup();
    const event = { toolName: "bash", input: { command: "rm -rf victim" } };
    h.c.cwd = join(homedir(), "task-a");
    await h.call(event, h.c);
    h.choices.push("1. B1: rm -rf victim", "Allow next exact call once");
    await h.command("pending", h.c);
    expect((await h.call(event, { ...h.c, cwd: join(homedir(), "task-b") })).block).toBe(true);
    expect(await h.call(event, h.c)).toBeUndefined();
  });
  test("an explicit denial is distinct from deferral", async () => {
    const h = setup();
    h.choices.push("Deny");
    expect((await h.call({ toolName: "bash", input: { command: "git push" } }, h.c)).reason).toContain("user denied");
  });
  test("scope is user-command-only, operation-limited and cleared on session start", async () => {
    const h = setup();
    const root = mkdtempSync(join(homedir(), ".policy-scope-test-"));
    h.c.cwd = join(homedir(), "unrelated-checkout");
    try {
      await h.command(`auto ${root}`, h.c);
      const event = { toolName: "write", input: { path: join(root, "notes.txt") } };
      expect(await h.call(event, h.c)).toBeUndefined();
      expect(h.prompts).toHaveLength(0);
      expect((await h.call({ toolName: "write", input: { path: join(root, "config.toml") } }, h.c)).block).toBe(true);
      expect((await h.call({ toolName: "bash", input: { command: "git push" } }, h.c)).block).toBe(true);
      h.start({}, h.c);
      expect((await h.call(event, h.c)).block).toBe(true);
      await h.command(`auto ${homedir()}`, h.c);
      expect(h.notifications.at(-1)).toContain("not the whole account");
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

describe("battery admission", () => {
  test("recognises commands that compile, and nothing else", () => {
    for (const [command, expected] of [
      ["cargo test -q", "cargo test"],
      ["cd crates/x && cargo clippy --all-targets", "cargo clippy"],
      ["bun run build", "bun run"],
      ["npm test", "npm test"],
      ["FOO=1 xcodebuild -scheme Sjel", "xcodebuild"],
      ["git status", null],
      ["ls -la && rg cargo", null],
      ["tools/cargo-hermetic test --workspace", null],
      ["sjel test", null],
    ] as const) {
      expect(heavyCommand(command)).toBe(expected);
    }
  });

  test("a command carrying the exception is never deferred", () => {
    expect(powerDeferral("SJEL_ALLOW_LOCAL_BUILD=1 cargo test", "/nonexistent")).toBeNull();
    expect(powerDeferral("git status", "/nonexistent")).toBeNull();
  });

  test("a deferred command is refused with the reason and the way out", () => {
    const root = mkdtempSync(join(tmpdir(), "policy-power-"));
    try {
      mkdirSync(join(root, "tools"), { recursive: true });
      writeFileSync(join(root, "tools/local-build"), "#!/bin/sh\necho 'deferred by runtime profile: On the go' >&2\nexit 3\n", { mode: 0o755 });
      const reason = powerDeferral("cargo build", root);
      expect(reason).toContain("cargo build is deferred");
      expect(reason).toContain("SJEL_ALLOW_LOCAL_BUILD=1");
      writeFileSync(join(root, "tools/local-build"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      expect(powerDeferral("cargo build", root)).toBeNull();
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
