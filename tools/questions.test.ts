import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const install = process.env.PI_MANAGED_INSTALL_ROOT ?? join(homedir(), ".pi/agent/install");
const marker = join(install, "current-version");
const modules = existsSync(marker)
  ? join(install, "releases", readFileSync(marker, "utf8").trim(), "node_modules")
  : undefined;
let fixture: string;
let register: (api: unknown) => void;

if (!modules) console.log("questions: skipped — managed Pi installation unavailable");

describe.skipIf(!modules)("stateless ask rounds", () => {
  beforeAll(async () => {
    fixture = mkdtempSync(join(tmpdir(), "questions-"));
    symlinkSync(modules!, join(fixture, "node_modules"), "dir");
    copyFileSync(join(import.meta.dir, "../Packs/knowledge/extensions/questions.ts"), join(fixture, "questions.ts"));
    register = (await import(join(fixture, "questions.ts"))).default;
  });
  afterAll(() => { if (fixture) rmSync(fixture, { recursive: true, force: true }); });

  function clock() {
    const set = globalThis.setTimeout;
    const clear = globalThis.clearTimeout;
    let now = 0;
    let next = 0;
    const timers = new Map<number, { at: number; callback: () => void }>();
    globalThis.setTimeout = ((callback: () => void, ms: number) => {
      const id = ++next;
      timers.set(id, { at: now + ms, callback });
      return id;
    }) as unknown as typeof setTimeout;
    globalThis.clearTimeout = ((id: number) => { timers.delete(id); }) as unknown as typeof clearTimeout;
    return {
      timers,
      advance(ms: number) {
        now += ms;
        for (const [id, timer] of [...timers]) if (timer.at <= now) {
          timers.delete(id);
          timer.callback();
        }
      },
      restore() { globalThis.setTimeout = set; globalThis.clearTimeout = clear; },
    };
  }

  function harness() {
    const tools = new Map<string, any>();
    const commands = new Map<string, any>();
    const messages: any[] = [];
    const statuses: (string | undefined)[] = [];
    register({
      registerTool: (tool: any) => tools.set(tool.name, tool),
      registerCommand: (name: string, command: any) => commands.set(name, command),
      sendMessage: (...message: any[]) => messages.push(message),
    });
    let widget: any;
    let doneCount = 0;
    let editorCount = 0;
    const theme = new Proxy({}, { get: () => (...args: any[]) => args.at(-1) });
    const ctx: any = {
      mode: "tui",
      ui: {
        custom: (factory: any) => new Promise((resolve) => {
          widget = factory({ requestRender() {}, terminal: { rows: 30, cols: 120 } }, theme, {}, (value: unknown) => {
            doneCount++;
            resolve(value);
          });
        }),
        setStatus: (_key: string, value: string | undefined) => statuses.push(value),
        notify() {},
        editor() { editorCount++; throw Error("RPC editor must not open"); },
      },
    };
    return {
      ctx, tools, commands, messages, statuses,
      get widget() { return widget; },
      get doneCount() { return doneCount; },
      get editorCount() { return editorCount; },
      run: (params: unknown, signal?: AbortSignal) => tools.get("ask").execute("ask-id", params, signal, undefined, ctx),
      keys(...keys: string[]) { for (const key of keys) widget.handleInput(key); },
    };
  }
  const q = (id: string, allowNote = true) => ({ id, prompt: `Choose ${id}?`, options: [{ label: "a" }, { label: "b" }], recommendedIndex: 0, allowNote });

  test("no idle timer, pending status or resume command is registered", async () => {
    const c = clock();
    try {
      const h = harness();
      expect(h.commands.has("decisions")).toBe(false);
      const result = h.run({ topic: "scope", questions: [q("q", false)] });
      expect(c.timers.size).toBe(0);
      c.advance(600_000);
      expect(h.doneCount).toBe(0);
      h.keys("\r");
      expect((await result).details.answers[0].label).toBe("a");
      expect(h.statuses).toEqual([]);
      expect(h.messages).toEqual([]);
    } finally { c.restore(); }
  });

  test("cancellation preserves submitted answers but grants no unanswered choices", async () => {
    const h = harness();
    const result = h.run({ topic: "partial", questions: [q("first", false), q("second")] });
    h.keys("\r", "\x1b");
    const r = await result;
    expect(r.details.cancelled).toBe(true);
    expect(r.details.answers.map((a: any) => a.id)).toEqual(["first"]);
    expect(r.details.unresolvedIds).toEqual(["second"]);
    expect(r.details.deferred).toBeUndefined();
    expect(r.content[0].text).toContain("owning plan");
    expect(r.content[0].text).not.toContain("/decisions");
  });

  test("reusing a topic opens a fresh round rather than replaying cached answers", async () => {
    const h = harness();
    const params = { topic: "same", questions: [q("q", false)] };
    const first = h.run(params);
    h.keys("\r");
    expect((await first).details.answers[0].label).toBe("a");
    const second = h.run(params);
    expect(h.doneCount).toBe(1);
    h.keys("\x1b[B", "\r");
    expect((await second).details.answers[0].label).toBe("b");
    expect(h.doneCount).toBe(2);
  });

  test("abort and dispose cancel without approval or cached state", async () => {
    for (const end of ["abort", "dispose", "already-aborted"]) {
      const h = harness();
      const controller = new AbortController();
      if (end === "already-aborted") controller.abort();
      const result = h.run({ topic: end, questions: [q("q")] }, controller.signal);
      if (end === "abort") controller.abort();
      if (end === "dispose") h.widget.dispose();
      const r = await result;
      expect(r.details.cancelled).toBe(true);
      expect(r.details.answers).toEqual([]);
      expect(r.details.deferred).toBeUndefined();
      expect(h.statuses).toEqual([]);
    }
  });

  test("RPC asks have no idle deadline and validate submitted answers", async () => {
    const old = process.env.PI_QUESTIONS_CLIENT;
    process.env.PI_QUESTIONS_CLIENT = "pi-questions/v1";
    try {
      const h = harness();
      h.ctx.mode = "rpc";
      const requests: any[] = [];
      h.ctx.ui.input = async (title: string, raw: string) => {
        requests.push({ title, params: JSON.parse(raw) });
        return JSON.stringify({ cancelled: false, answers: [{
          id: "q", selectedIndex: 1, label: "b", freeText: null, note: "reason",
          followedRecommendation: false, skipped: false,
        }] });
      };
      const result = await h.run({ topic: "rpc", questions: [q("q")] });
      expect(requests[0].params.idleMs).toBeUndefined();
      expect(requests[0].title).toBe("pi-questions/v1 ask");
      expect(result.details.answers[0]).toMatchObject({ label: "b", note: "reason" });
      expect(h.commands.has("decisions")).toBe(false);
      h.ctx.ui.input = async () => undefined;
      const cancelled = await h.run({ topic: "rpc", questions: [q("q")] });
      expect(cancelled.details.cancelled).toBe(true);
      expect(cancelled.details.answers).toEqual([]);
    } finally {
      if (old === undefined) delete process.env.PI_QUESTIONS_CLIENT;
      else process.env.PI_QUESTIONS_CLIENT = old;
    }
  });

  test("duplicate question IDs are rejected", async () => {
    const h = harness();
    await expect(h.run({ topic: "duplicate", questions: [q("q"), q("q")] })).rejects.toThrow("duplicate question");
  });
});
