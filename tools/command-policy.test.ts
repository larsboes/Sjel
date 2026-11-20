// Command policy (docs/plans/agent.md §3) for pi: every case in
// Packs/security/extensions/command-policy/cases.toml, plus the extension's card behaviour.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { classify, classifyWrite, resolvePath, type Scope } from "../Packs/security/extensions/command-policy/classify.ts";
import commandPolicy from "../Packs/security/extensions/command-policy/index.ts";
import { scopedCommand, scopedWrite } from "../Packs/security/extensions/command-policy/scoped.ts";
import { heavyCommand, powerDeferral } from "../Packs/security/extensions/command-policy/power.ts";
import { loadMarkdownRoots, scopedMarkdownWrite } from "../Packs/security/extensions/command-policy/config.ts";

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

describe("persistent machine-local Markdown scopes", () => {
  let root: string;
  let overlay: string;
  let previousOverlay: string | undefined;
  let previousPersonal: string | undefined;
  const s: Scope = { cwd: join(homedir(), "unrelated-checkout"), home: homedir(), writable: [] };
  const configure = (value: unknown) => writeFileSync(join(overlay, "config", "command-policy.json"), JSON.stringify(value));
  beforeEach(() => {
    previousOverlay = process.env.SJEL_OVERLAY_ROOT;
    previousPersonal = process.env.SJEL_PERSONAL_ROOT;
    root = mkdtempSync(join(homedir(), ".policy-markdown-test-"));
    overlay = mkdtempSync(join(tmpdir(), "policy-config-"));
    mkdirSync(join(overlay, "config"));
    process.env.SJEL_OVERLAY_ROOT = overlay;
    delete process.env.SJEL_PERSONAL_ROOT;
  });
  afterEach(() => {
    if (previousOverlay === undefined) delete process.env.SJEL_OVERLAY_ROOT;
    else process.env.SJEL_OVERLAY_ROOT = previousOverlay;
    if (previousPersonal === undefined) delete process.env.SJEL_PERSONAL_ROOT;
    else process.env.SJEL_PERSONAL_ROOT = previousPersonal;
    rmSync(root, { recursive: true, force: true });
    rmSync(overlay, { recursive: true, force: true });
  });
  function setup(hasUI = true) {
    const handlers = new Map<string, (...args: any[]) => any>();
    let command: any;
    const notifications: string[] = [];
    let prompts = 0;
    commandPolicy({
      on: (name: string, fn: any) => handlers.set(name, fn),
      registerCommand: (_name: string, value: any) => { command = value.handler; },
      getSettings: () => { throw new Error("Merged project settings are not permission authority"); },
    } as never);
    const c = { cwd: s.cwd, hasUI, ui: {
      notify: (text: string) => notifications.push(text), setStatus: () => {},
      select: async () => { prompts++; return "Deny"; },
    } };
    const start = () => handlers.get("session_start")!({}, c);
    start();
    return { call: (toolName: string, path: string) => handlers.get("tool_call")!({ toolName, input: { path } }, c),
      shell: (command: string, toolName = "bash") => handlers.get("tool_call")!({ toolName, input: { command, cmd: command } }, c),
      command: (args: string) => command(args, c), start, notifications, prompts: () => prompts };
  }
  test("missing config grants nothing; overlay alias and canonical roots are supported", () => {
    expect(loadMarkdownRoots(s)).toEqual([]);
    configure({ markdown_write_roots: [root, root] });
    expect(loadMarkdownRoots(s)).toEqual([resolvePath(root, s)]);
    process.env.SJEL_PERSONAL_ROOT = overlay;
    delete process.env.SJEL_OVERLAY_ROOT;
    expect(loadMarkdownRoots(s)).toEqual([resolvePath(root, s)]);
    delete process.env.SJEL_PERSONAL_ROOT;
    expect(loadMarkdownRoots(s)).toEqual([]);
  });
  test("malformed config, broad roots, missing roots and unknown keys fail closed", async () => {
    for (const value of [null, [], {}, { markdown_write_roots: "bad" },
      { markdown_write_roots: [root, 1] }, { markdown_write_roots: ["relative"] },
      { markdown_write_roots: [root, "/"] }, { markdown_write_roots: [homedir()] },
      { markdown_write_roots: [join(root, "missing")] },
      { markdown_write_roots: [root], shell_roots: [root] }]) {
      configure(value);
      expect(() => loadMarkdownRoots(s)).toThrow();
      const h = setup();
      expect(h.notifications[0]).toContain("no persistent Markdown grants");
      expect((await h.call("write", join(root, "note.md"))).block).toBe(true);
    }
    writeFileSync(join(overlay, "config", "command-policy.json"), "{broken");
    expect(() => loadMarkdownRoots(s)).toThrow();
    rmSync(join(overlay, "config", "command-policy.json"));
    mkdirSync(join(overlay, "config", "command-policy.json"));
    expect(() => loadMarkdownRoots(s)).toThrow();
    expect(setup().notifications[0]).toContain("invalid or unreadable");
    rmSync(join(overlay, "config", "command-policy.json"), { recursive: true });
    configure({ markdown_write_roots: [] });
    expect(loadMarkdownRoots(s)).toEqual([]);
    symlinkSync(homedir(), join(root, "home-alias"));
    configure({ markdown_write_roots: [join(root, "home-alias")] });
    expect(() => loadMarkdownRoots(s)).toThrow();
    writeFileSync(join(root, "file"), "");
    configure({ markdown_write_roots: [join(root, "file")] });
    expect(() => loadMarkdownRoots(s)).toThrow();
  });
  test("only direct Markdown write/edit gain approval; shell and protected targets still ask", async () => {
    configure({ markdown_write_roots: [root] });
    const h = setup();
    expect(await setup(false).call("write", join(root, "headless.md"))).toBeUndefined();
    for (const tool of ["write", "edit"]) {
      expect(await h.call(tool, join(root, "nested", "note.md"))).toBeUndefined();
      for (const path of ["note.txt", "note.md.ts", "secrets/note.md", "extensions/note.md", "hooks/note.md", "agents/note.md", "command-policy/note.md", ".ssh/note.md", "AGENTS.md", "CLAUDE.md", "SKILL.md", "GEMINI.md", "copilot-instructions.md", ".obsidian/prompts/system.md", ".pi/note.md", "config/policy.md", "prompts/system.md"]) {
        expect((await h.call(tool, join(root, path))).block).toBe(true);
      }
      expect((await h.call(tool, `${root}-sibling/note.md`)).block).toBe(true);
      expect((await h.call(tool, join(root, "..", "outside.md"))).block).toBe(true);
    }
    for (const shell of ["bash", "vault_exec"]) {
      expect((await h.shell(`touch '${root}/note.md'`, shell)).block).toBe(true);
      expect((await h.shell(`cp -n source '${root}/note.md'`, shell)).block).toBe(true);
    }
  });
  test("persistent Markdown grants reject canonical policy aliases", async () => {
    configure({ markdown_write_roots: [root] });
    mkdirSync(join(root, ".obsidian", "prompts"), { recursive: true });
    writeFileSync(join(root, "AGENTS.md"), "policy");
    writeFileSync(join(root, ".obsidian", "prompts", "system.md"), "policy");
    symlinkSync(join(root, "AGENTS.md"), join(root, "ordinary.md"));
    symlinkSync(join(root, ".obsidian", "prompts"), join(root, "ordinary-directory"));
    const h = setup();
    for (const tool of ["write", "edit"]) {
      for (const path of ["ordinary.md", "ordinary-directory/system.md"]) {
        expect((await h.call(tool, join(root, path))).block).toBe(true);
      }
    }
  });
  test("canonical containment rejects symlink escapes, suffix aliases and dangling links", async () => {
    const outside = join(homedir(), "policy-outside-does-not-exist");
    symlinkSync(homedir(), join(root, "escape"));
    symlinkSync(outside, join(root, "dangling.md"));
    symlinkSync(outside, join(root, "dangling-directory"));
    writeFileSync(join(root, "script.ts"), "");
    symlinkSync(join(root, "script.ts"), join(root, "alias.md"));
    const roots = [resolvePath(root, s)];
    for (const path of ["escape/note.md", "dangling.md", "dangling-directory/note.md", "alias.md"]) {
      expect(scopedMarkdownWrite(join(root, path), s, roots)).toBe(false);
    }
    configure({ markdown_write_roots: [root] });
    const h = setup();
    expect((await h.call("write", join(root, "escape", "note.md"))).block).toBe(true);
    const task = join(root, "task");
    mkdirSync(task);
    const pinned = [resolvePath(task, s)];
    renameSync(task, join(root, "old-task"));
    symlinkSync(homedir(), task);
    expect(scopedMarkdownWrite(join(task, "note.md"), s, pinned)).toBe(false);
  });
  test("grants reload across instances; off suppresses persistent roots without changing config", async () => {
    configure({ markdown_write_roots: [root] });
    const h = setup();
    const note = join(root, "note.md");
    expect(await h.call("write", note)).toBeUndefined();
    expect(h.prompts()).toBe(0);
    await h.command("status");
    expect(h.notifications.at(-1)).toContain(`Persistent Markdown roots (active): ${resolvePath(root, s)}`);
    await h.command("off");
    await h.command("status");
    expect(h.notifications.at(-1)).toContain("suppressed until reload/session change");
    expect((await h.call("write", note)).block).toBe(true);
    await h.command(`auto ${root}`);
    expect(await h.call("edit", note)).toBeUndefined();
    await h.command("off");
    expect((await h.call("edit", note)).block).toBe(true);
    expect(await setup().call("write", note)).toBeUndefined();
    h.start();
    expect(await h.call("write", note)).toBeUndefined();
    expect(loadMarkdownRoots(s)).toEqual([resolvePath(root, s)]);
    configure({ markdown_write_roots: [root, "/"] });
    h.start();
    expect((await h.call("write", note)).block).toBe(true);
    expect(h.notifications.at(-1)).toContain("no persistent Markdown grants");
  });
});

describe("immediate permissions and scope lifecycle", () => {
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
    const prompts: { options: string[]; timeout?: number }[] = [];
    const notifications: string[] = [];
    const c = { cwd: homedir(), hasUI: true, ui: {
      select: async (_title: string, options: string[], opts: { timeout?: number }) => {
        prompts.push({ options, timeout: opts.timeout });
        return choices.shift();
      },
      setStatus: () => {}, notify: (text: string) => notifications.push(text),
    } };
    return { call: handlers.get("tool_call")!, start: handlers.get("session_start")!, command: commands.get("permissions")!.handler, choices, prompts, notifications, messages, c };
  }
  test("cancellation stores no action, has no timeout or defer option, and grants nothing", async () => {
    const h = setup();
    const event = { toolName: "bash", input: { command: "git push origin main" } };
    const result = await h.call(event, h.c);
    expect(result.block).toBe(true);
    expect(result.reason).toContain("approval cancelled");
    expect(result.reason).toContain("No pending action is stored");
    expect(h.prompts[0].timeout).toBeUndefined();
    expect(h.prompts[0].options).not.toContain("Defer");
    await h.command("pending", h.c);
    expect(h.prompts).toHaveLength(1);
    expect(h.messages).toEqual([]);
    expect(h.notifications.at(-1)).not.toContain("deferred");
    h.choices.push("Allow once");
    expect(await h.call(event, h.c)).toBeUndefined();
    expect((await h.call(event, h.c)).block).toBe(true);
    expect(h.prompts).toHaveLength(3);
  });

  test("an abort grants nothing", async () => {
    const h = setup();
    const controller = new AbortController();
    controller.abort();
    h.choices.push("Allow this exact call for this session");
    const event = { toolName: "bash", input: { command: "git push" } };
    expect((await h.call(event, { ...h.c, signal: controller.signal })).reason).toContain("aborted");
    expect((await h.call(event, h.c)).block).toBe(true);
    expect(h.messages).toEqual([]);
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

  test("off invalidates outstanding approvals", async () => {
    const h = setup();
    let resolve!: (choice: string) => void;
    const event = { toolName: "bash", input: { command: "git push" } };
    const call = h.call(event, { ...h.c, ui: { ...h.c.ui, select: () => new Promise<string>((done) => { resolve = done; }) } });
    await h.command("off", h.c);
    resolve("Allow this exact call for this session");
    expect((await call).block).toBe(true);
    expect((await h.call(event, h.c)).block).toBe(true);
  });

  test("session approval is bound to the exact call and working directory", async () => {
    const h = setup();
    const event = { toolName: "bash", input: { command: "git push" } };
    h.c.cwd = join(homedir(), "task-a");
    h.choices.push("Allow this exact call for this session");
    expect(await h.call(event, h.c)).toBeUndefined();
    expect(await h.call(event, h.c)).toBeUndefined();
    expect(h.prompts).toHaveLength(1);
    expect((await h.call(event, { ...h.c, cwd: join(homedir(), "task-b") })).block).toBe(true);
    h.start({}, h.c);
    expect((await h.call(event, h.c)).block).toBe(true);
  });

  test("Block rules allow only the current call, never a session grant", async () => {
    const h = setup();
    const event = { toolName: "bash", input: { command: "rm -rf victim" } };
    h.choices.push("Allow once");
    expect(await h.call(event, h.c)).toBeUndefined();
    expect(h.prompts[0].options).toEqual(["Deny", "Allow once"]);
    expect((await h.call(event, h.c)).block).toBe(true);
  });

  test("an explicit denial is not approval or cancellation", async () => {
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
