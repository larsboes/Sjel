import { describe, expect, it } from "vitest";
import { resolveModel, type ModelEntry } from "../src/model-resolver.js";

/**
 * The regression this file exists for: a `provider/model` pin must resolve under
 * that provider. OpenRouter publishes the upstream provider inside the model id,
 * so its entry for Claude Haiku is `id: "anthropic/claude-haiku-4.5"` — which
 * normalizes to exactly the query `"anthropic/claude-haiku-4-5"`. Before the
 * scoping in `resolveModel` step 2, that exact hit scored 100 and beat the real
 * `claude-haiku-4-5` entry on every other provider, so the built-in `Explore`
 * agent's pin (`anthropic/claude-haiku-4-5`) silently billed OpenRouter and died
 * on its balance instead of resolving to a reachable Haiku.
 */

const MODELS: ModelEntry[] = [
  { provider: "openrouter", id: "anthropic/claude-haiku-4.5", name: "Claude Haiku 4.5" },
  { provider: "opencode", id: "claude-haiku-4-5", name: "Claude Haiku 4.5" },
  { provider: "opencode-go", id: "deepseek-v4.1-flash", name: "DeepSeek V4.1 Flash" },
];

function registryOf(models: ModelEntry[]) {
  return {
    getAll: () => models,
    getAvailable: () => models,
    find: (provider: string, id: string) =>
      models.find(m => m.provider === provider && m.id === id),
  };
}

describe("resolveModel", () => {
  it("resolves a provider-prefixed pin under that provider, not on a gateway whose id embeds the prefix", () => {
    const resolved = resolveModel("anthropic/claude-haiku-4-5", registryOf(MODELS));

    expect(typeof resolved).not.toBe("string");
    expect(resolved.provider).toBe("opencode");
    expect(resolved.id).toBe("claude-haiku-4-5");
  });

  it("still resolves an OpenRouter id when the gateway is named explicitly", () => {
    const resolved = resolveModel("openrouter/anthropic/claude-haiku-4.5", registryOf(MODELS));

    expect(typeof resolved).not.toBe("string");
    expect(resolved.provider).toBe("openrouter");
    expect(resolved.id).toBe("anthropic/claude-haiku-4.5");
  });

  it("keeps a same-provider fuzzy match after scoping", () => {
    const resolved = resolveModel("opencode-go/deepseek", registryOf(MODELS));

    expect(typeof resolved).not.toBe("string");
    expect(resolved.provider).toBe("opencode-go");
    expect(resolved.id).toBe("deepseek-v4.1-flash");
  });

  it("keeps a bare query fuzzy across providers", () => {
    const resolved = resolveModel("haiku", registryOf(MODELS));

    expect(typeof resolved).not.toBe("string");
    expect(resolved.id).toBe("claude-haiku-4-5");
  });

  it("reports an unresolvable pin instead of returning a model", () => {
    const only = registryOf([MODELS[2]]);
    const resolved = resolveModel("anthropic/claude-haiku-4-5", only);

    expect(typeof resolved).toBe("string");
    expect(resolved).toContain("Model not found");
  });
});
