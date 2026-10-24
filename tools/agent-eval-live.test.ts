// Both live adapters against one scripted OpenAI-compatible endpoint: no model, no cost.
// Pi needs its installed SDK and sjel-agent-eval needs a build, so both are opt-in:
//   cargo build -p sjel-agent --bin sjel-agent-eval
//   SJEL_AGENT_EVAL_LIVE_ADAPTERS=1 bun --no-env-file test tools/agent-eval-live.test.ts
import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { validateSuite, type EvalCase } from "./lib/agent-eval.ts";
import { liveSupported, runPi, runSjel, SYSTEM_PROMPT, validateManifest, type RunManifest } from "./lib/agent-eval-live.ts";

const enabled = process.env.SJEL_AGENT_EVAL_LIVE_ADAPTERS === "1";
const suite = validateSuite(JSON.parse(readFileSync(new URL("../research/benchmarks/agent-harness/cases.json", import.meta.url), "utf8")));
const byId = (id: string) => suite.cases.find((c) => c.id === id)! as EvalCase;
const repo = new URL("..", import.meta.url).pathname.replace(/\/$/, "");

/** A chat-completions endpoint that plays a fixed list of turns and records every request. */
function scriptedEndpoint(turns: ({ tool: string; args: unknown } | { text: string })[]) {
  const requests: any[] = [];
  const server = Bun.serve({
    port: 0,
    async fetch(req) {
      const body = await req.json();
      requests.push(body);
      const turn = turns[Math.min(requests.length - 1, turns.length - 1)];
      const chunk = (delta: unknown, finish: string | null) =>
        `data: ${JSON.stringify({ id: "r", object: "chat.completion.chunk", model: "m", choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;
      const usage = `data: ${JSON.stringify({ id: "r", object: "chat.completion.chunk", model: "m", choices: [], usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 } })}\n\n`;
      const events = "tool" in turn
        ? chunk({ role: "assistant", tool_calls: [{ index: 0, id: `call_${requests.length}`, type: "function", function: { name: turn.tool, arguments: JSON.stringify(turn.args) } }] }, null) + chunk({}, "tool_calls")
        : chunk({ role: "assistant", content: turn.text }, null) + chunk({}, "stop");
      return new Response(events + usage + "data: [DONE]\n\n", { headers: { "content-type": "text/event-stream" } });
    },
  });
  return { url: `http://127.0.0.1:${server.port}/v1`, requests, stop: () => server.stop(true) };
}

function fixture(url: string) {
  const dir = mkdtempSync(join(tmpdir(), "agent-eval-live-"));
  writeFileSync(join(dir, "auth.json"), "{}");
  writeFileSync(join(dir, "models.json"), JSON.stringify({ providers: { mock: {
    baseUrl: url, api: "openai-completions", apiKey: "mock-key",
    compat: { supportsDeveloperRole: false, supportsReasoningEffort: false, supportsUsageInStreaming: true, maxTokensField: "max_tokens" },
    models: [{ id: "m", name: "mock", reasoning: false, input: ["text"], contextWindow: 32_000, maxTokens: 1_000, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }],
  } } }));
  writeFileSync(join(dir, "inference.json"), JSON.stringify({ backends: { local: { api: "openai", base_url: url } }, roles: { coding: { backend: "local", model: "m" } } }));
  const manifest: RunManifest = {
    cases: ["C01"], harnesses: ["pi", "sjel"], repeats: 1, max_turns: 8, timeout_ms: 30_000, output_dir: join(dir, "out"),
    pi: { provider: "mock", model: "m", thinking: "off", config_dir: dir },
    sjel: { role: "coding", binary: join(process.env.CARGO_TARGET_DIR ?? join(process.env.HOME!, ".cargo-target"), "debug", "sjel-agent-eval"), inference_config: join(dir, "inference.json") },
  };
  return { dir, manifest };
}

const toolNames = (request: any) => (request.tools ?? []).map((t: any) => t.function.name).sort();
const systemText = (request: any) => request.messages.find((msg: any) => msg.role === "system")?.content;

describe("run manifest", () => {
  test("names every limit and keeps traces out of the repository", () => {
    const { manifest, dir } = fixture("http://127.0.0.1:1/v1");
    expect(validateManifest(manifest, repo).cases).toEqual(["C01"]);
    expect(() => validateManifest({ ...manifest, output_dir: join(repo, "runs") }, repo)).toThrow("outside the repository");
    expect(() => validateManifest({ ...manifest, repeats: 0 }, repo)).toThrow("repeats");
    expect(() => validateManifest({ ...manifest, harnesses: ["codex"] }, repo)).toThrow("harnesses");
    rmSync(dir, { recursive: true });
  });

  test("cases that need sandboxed Python are not offered live", () => {
    expect(suite.cases.filter((c) => !liveSupported(c as EvalCase)).map((c) => c.id).sort()).toEqual(["C02", "R01"]);
  });
});

describe.skipIf(!enabled)("live adapters against a scripted endpoint", () => {
  test("Pi and Sjel see the same tools and system prompt, and the broker records both", async () => {
    const turns = [
      { tool: "read", args: { path: "src/session.rs" } },
      { tool: "read", args: { path: "../cases.json" } },
      { text: "Recovery is described at src/session.rs:1." },
    ];
    for (const run of [runPi, (c: EvalCase, m: RunManifest) => runSjel(c, m, "test")]) {
      const endpoint = scriptedEndpoint(turns);
      const { dir, manifest } = fixture(endpoint.url);
      try {
        const { meta, broker, answer } = await run(byId("C01"), manifest);
        expect(meta.error).toBeNull();
        expect(meta.termination).toBe("final");
        expect(answer).toBe("Recovery is described at src/session.rs:1.");
        expect(endpoint.requests.length).toBe(3);
        // No harness tool beyond the broker's, and the shared system prompt reached the model.
        for (const request of endpoint.requests) expect(toolNames(request)).toEqual(["list", "read", "search"]);
        expect(systemText(endpoint.requests[0])).toContain(SYSTEM_PROMPT);
        const report = broker.report(answer, "live_harness");
        expect(report.calls.map((call) => call.status)).toEqual(["ok", "denied"]);
        expect(report.calls[0].content).toBe(byId("C01").input.files["src/session.rs"].content);
        expect(report.policy_failures).toEqual([1]);
        // The scripted endpoint reports 10 + 5 tokens per request; both harnesses must carry it.
        expect(Array.isArray(meta.usage) && meta.usage.length).toBe(3);
      } finally {
        endpoint.stop();
        rmSync(dir, { recursive: true });
      }
    }
  }, 60_000);

  test("R04's probe goes through the broker and spends its single attempt", async () => {
    const turns = [
      { tool: "probe", args: { x: 4, predictions: { H1: 0, H2: 1 } } },
      { tool: "probe", args: { x: 5, predictions: { H1: 1, H2: 0 } } },
      { text: "H2 survives." },
    ];
    for (const run of [runPi, (c: EvalCase, m: RunManifest) => runSjel(c, m, "test")]) {
      const endpoint = scriptedEndpoint(turns);
      const { dir, manifest } = fixture(endpoint.url);
      try {
        const { broker, answer } = await run(byId("R04"), { ...manifest, cases: ["R04"] });
        const report = broker.report(answer, "live_harness");
        expect(report.calls.map((call) => call.status)).toEqual(["ok", "denied"]);
        expect(report.behavior).toEqual({ status: "failed", probe_attempts: 2, probe_successes: 1 });
      } finally {
        endpoint.stop();
        rmSync(dir, { recursive: true });
      }
    }
  }, 60_000);
});
