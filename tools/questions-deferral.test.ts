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

if (!modules) console.log("questions-deferral: skipped — managed Pi installation unavailable");

describe.skipIf(!modules)("ask deferral", () => {
  beforeAll(async () => {
    fixture = mkdtempSync(join(tmpdir(), "questions-deferral-"));
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
      resume: () => commands.get("decisions").handler("", ctx),
      keys(...keys: string[]) { for (const key of keys) widget.handleInput(key); },
    };
  }
  const q = (id: string, allowNote = true) => ({ id, prompt: `Choose ${id}?`, options: [{ label: "a" }, { label: "b" }], recommendedIndex: 0, allowNote });

  test("idle resets on keystrokes, retains completed answers and note draft, then resumes unresolved only", async () => {
    const c = clock();
    try {
      const h = harness();
      const params = { topic: "partial", questions: [q("first", false), q("second")] };
      const result = h.run(params);
      expect(h.widget.render(120).join("\n")).toContain("defer after 60 sec idle");
      h.keys("\r", "2", "\r", ..."draft note");
      c.advance(59_999);
      h.keys("!");
      c.advance(59_999);
      expect(h.doneCount).toBe(0);
      c.advance(1);
      const r = await result;
      expect(r.details.deferred).toBe(true);
      expect(r.details.cancelled).toBe(false);
      expect(r.details.answers.map((a: any) => a.id)).toEqual(["first"]);
      expect(r.details.unresolvedIds).toEqual(["second"]);
      expect(r.details.draft).toEqual({ id: "second", phase: "note", optionIndex: 1, text: "draft note!" });
      expect(r.content[0].text).toContain("Continue independent work only");
      expect(h.statuses.at(-1)).toContain("1 decisions pending");
      expect(await h.run(params)).toBe(r);
      expect(await h.run({ ...params, questions: [params.questions[1]] })).toBe(r);
      expect(h.statuses.filter(Boolean)).toHaveLength(1);
      expect(h.doneCount).toBe(1);
      expect(c.timers.size).toBe(0);
      h.widget.dispose();
      expect(h.doneCount).toBe(1);
      const resume = h.resume();
      expect(h.widget.render(120).join("\n")).toContain("Choose second?");
      h.keys("\r");
      await resume;
      const [message, options] = h.messages.at(-1);
      expect(message.details.answers.map((a: any) => a.id)).toEqual(["first", "second"]);
      expect(message.details.answers[1].note).toBe("draft note!");
      expect(message.details.answers[1].label).toBe("b");
      expect(options.triggerTurn).toBe(false);
      expect(h.statuses.at(-1)).toBeUndefined();
      expect(c.timers.size).toBe(0);
    } finally { c.restore(); }
  });

  test("abort preserves free text draft, removes listener/timer, and skips remain open on resume", async () => {
    const c = clock();
    try {
      const h = harness();
      const controller = new AbortController();
      const remove = controller.signal.removeEventListener.bind(controller.signal);
      let removed = 0;
      controller.signal.removeEventListener = (...args) => { removed++; remove(...args); };
      const result = h.run({ topic: "abort", questions: [q("skip"), q("free")] }, controller.signal);
      h.keys("s", "o", ..."unfinished");
      controller.abort();
      const r = await result;
      expect(r.details.deferReason).toBe("aborted");
      expect(r.details.draft.text).toBe("unfinished");
      expect(r.details.draft.phase).toBe("free");
      expect(r.details.unresolvedIds).toEqual(["skip", "free"]);
      expect(removed).toBe(1);
      expect(c.timers.size).toBe(0);
      h.widget.dispose();
      c.advance(60_000);
      expect(h.doneCount).toBe(1);
      const resume = h.resume();
      expect(h.widget.render(120).join("\n")).toContain("Choose skip?");
      h.keys("s");
      expect(h.widget.render(120).join("\n")).toContain("unfinished");
      h.keys("\r");
      await resume;
      expect(h.messages.at(-1)[0].details.answers[1].freeText).toBe("unfinished");
      expect(h.statuses.at(-1)).toContain("1 decisions pending");
      const finish = h.resume();
      h.keys("\r", "\r");
      await finish;
      expect(h.statuses.at(-1)).toBeUndefined();
    } finally { c.restore(); }
  });

  test("disposal and Escape settle once and clean up; pre-abort opens no widget", async () => {
    const c = clock();
    try {
      for (const how of ["dispose", "escape"]) {
        const h = harness();
        const result = h.run({ topic: how, questions: [q("q")] });
        if (how === "dispose") h.widget.dispose();
        else h.keys("\x1b");
        const r = await result;
        expect(r.details.deferReason).toBe(how === "dispose" ? "disposed" : undefined);
        expect(r.details.cancelled).toBe(how === "escape");
        expect(c.timers.size).toBe(0);
        h.widget.dispose();
        c.advance(60_000);
        expect(h.doneCount).toBe(1);
      }
      const h = harness();
      const controller = new AbortController();
      controller.abort();
      const r = await h.run({ topic: "before", questions: [q("q")] }, controller.signal);
      expect(r.details.deferReason).toBe("aborted");
      expect(h.widget).toBeUndefined();
      expect(c.timers.size).toBe(0);
    } finally { c.restore(); }
  });

  test("RPC reports unsupported native timeout/drafts instead of opening an uninterruptible editor", async () => {
    const old = process.env.PI_QUESTIONS_CLIENT;
    process.env.PI_QUESTIONS_CLIENT = "pi-questions/v1";
    try {
      const h = harness();
      h.ctx.mode = "rpc";
      const r = await h.run({ topic: "rpc", questions: [q("q")] });
      expect(r.details.deferReason).toBe("rpc-unsupported");
      expect(r.details.answers).toEqual([]);
      expect(r.content[0].text).toContain("cannot preserve an unsubmitted RPC draft");
      expect(h.editorCount).toBe(0);
    } finally {
      if (old === undefined) delete process.env.PI_QUESTIONS_CLIENT;
      else process.env.PI_QUESTIONS_CLIENT = old;
    }
  });

  test("overlapping rounds queue new questions and explicitly invalidate revised answers without reopening", async () => {
    const c = clock();
    try {
      const h = harness();
      const first = q("first", false);
      const old = q("old");
      const initial = h.run({ topic: "overlap", questions: [first, old] });
      h.keys("\r");
      c.advance(60_000);
      await initial;
      const updated = { ...first, options: [{ label: "revised-a" }, { label: "revised-b" }] };
      const r = await h.run({ topic: "overlap", questions: [updated, old, q("NEW", false)] });
      expect(r.details.answers).toEqual([]);
      expect(r.details.unresolvedIds).toEqual(["first", "old", "NEW"]);
      expect(r.content.map((v: any) => v.text).join("\n")).toContain("Revised question definitions: first");
      expect(h.doneCount).toBe(1);
      expect(c.timers.size).toBe(0);
      const resume = h.resume();
      expect(h.widget.render(120).join("\n")).toContain("revised-a");
      h.keys("\r", "\r", "\r", "\r");
      await resume;
      expect(h.messages.at(-1)[0].details.answers.map((a: any) => a.id)).toEqual(["first", "old", "NEW"]);
      expect(h.statuses.at(-1)).toBeUndefined();
    } finally { c.restore(); }
  });

  test("quiz and narrow have no idle timers", async () => {
    const c = clock();
    try {
      const h = harness();
      const quiz = h.tools.get("quiz").execute("quiz", { topic: "quiz", questions: [{ id: "q", prompt: "?", options: ["a", "b"], correctIndex: 0 }] }, undefined, undefined, h.ctx);
      expect(c.timers.size).toBe(0);
      h.keys("\x1b");
      await quiz;
      const narrow = h.tools.get("narrow").execute("narrow", { topic: "narrow", candidates: [{ id: "c", oneLine: "candidate" }] }, undefined, undefined, h.ctx);
      expect(c.timers.size).toBe(0);
      h.keys("\x1b");
      await narrow;
    } finally { c.restore(); }
  });
});
