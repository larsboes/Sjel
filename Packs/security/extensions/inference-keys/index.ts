/**
 * inference-keys — drive pi's API-key providers from the Bitwarden vault.
 *
 * Every provider listed in CATALOG gets its `apiKey` pointed at
 * `bw-key.mjs <slug>`, so pi asks the vault for the key instead of reading an
 * environment variable or `auth.json`. Nothing is copied into the environment,
 * and the key never appears in a tool call or in this file.
 *
 * The catalog is split in two, and the distinction carries the design:
 *
 *   builtin: true   pi ships this provider and its curated model metadata.
 *                   Registering auth alone leaves that catalog intact, so
 *                   Groq's, NVIDIA's and Gemini's model lists stay pi's own.
 *
 *   builtin: false  pi has no such provider, so the entry supplies baseUrl,
 *                   api and a starter model list, and `discover` refreshes the
 *                   list from the provider's own /models endpoint. Because
 *                   extension `models` REPLACE a provider's models, only the
 *                   non-builtin entries ever carry a model list.
 *
 * Eligibility is decided by the vault, not by this file: the extension asks
 * bw-key.mjs which `inference-<slug>-api-key` Secure Notes actually exist and
 * registers only those. Adding a key in Bitwarden is therefore the whole
 * installation step for a new provider — no edit here, no restart of anything
 * but pi.
 *
 * Commands: /bw-status, /bw-unlock, /bw-refresh.
 */

import { execFile } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { RefreshModelsContext } from "@earendil-works/pi-ai";

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

interface ModelDef {
	id: string;
	name: string;
	reasoning: boolean;
	input: ("text" | "image")[];
	contextWindow: number;
	maxTokens: number;
	cost: { input: number; output: number; cacheRead: number; cacheWrite: number };
}

interface CatalogEntry {
	/** pi provider id; must match the built-in id when `builtin` is set. */
	id: string;
	name: string;
	/** Vault slug: the item is `inference-<slug>-api-key`. */
	slug: string;
	builtin?: boolean;
	/** Exact item name when it does not follow the convention. */
	vaultItem?: string;
	keyUrl?: string;
	baseUrl?: string;
	api?: string;
	/** Applied to every model this entry contributes; extension provider config
	 * has no provider-level compat, so it is merged per model. */
	compat?: Record<string, unknown>;
	models?: ModelDef[];
	/** OpenAI-compatible model listing, for models newer than this file. */
	discover?: { url: string; auth?: boolean };
	note?: string;
}

/**
 * An entry pi cannot supply itself, so this file carries the endpoint. Those are
 * the only entries whose model records are built here, and a model record pi
 * persists has to be a complete one — `api` and `baseUrl` included, not just the
 * per-model facts. The flat catalog type cannot express "has a backend", which is
 * why this narrower one exists: assert the invariant once, at registration, and
 * the rest of the file gets a type that says what is already true.
 */
interface ServedEntry extends CatalogEntry {
	baseUrl: string;
	api: string;
}

function servedEntry(entry: CatalogEntry): ServedEntry | undefined {
	return typeof entry.baseUrl === "string" && typeof entry.api === "string" ? (entry as ServedEntry) : undefined;
}

const FREE = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 };

/** The output cap used whenever the catalog does not know better. See `m()`. */
const DEFAULT_MAX_TOKENS = 8192;

/**
 * Terse model constructor: only the facts that actually differ per model.
 *
 * `maxTokens` defaults low on purpose. pi sends it straight through as
 * `max_completion_tokens`, and a provider whose real output cap is smaller
 * rejects the whole request (Cohere answers 400 TOO_MANY_TOKENS for anything
 * above 8192). An over-generous cap is a hard failure that pi reports as a bare
 * status code, while a low one only shortens replies, so the default stays
 * conservative and the entries that can take more say so.
 */
function m(
	id: string,
	contextWindow: number,
	reasoning: boolean,
	opts: { image?: boolean; maxTokens?: number } = {},
): ModelDef {
	return {
		id,
		name: id,
		reasoning,
		input: opts.image ? ["text", "image"] : ["text"],
		contextWindow,
		maxTokens: opts.maxTokens ?? DEFAULT_MAX_TOKENS,
		cost: { ...FREE },
	};
}

/** Third-party gateways are OpenAI-shaped but need the `system` role and
 * generally reject `reasoning_effort`. A model that thinks on its own still
 * does; pi simply does not steer it. */
const GATEWAY_COMPAT = { supportsDeveloperRole: false, supportsReasoningEffort: false };
/** Ollama's OpenAI layer does map `reasoning_effort` for thinking models. */
const OLLAMA_COMPAT = { supportsDeveloperRole: false, supportsReasoningEffort: true };

/**
 * Model facts come from the provider directory this was built against
 * (awesome-freellm-apis / freellm.net, refreshed 2026-09-11) and from
 * ollama.com's public /api/tags. Free tiers churn monthly, so treat
 * `contextWindow` and `reasoning` as the fields that need a hand edit, and
 * prefer `discover` where a provider exposes /models.
 */
const CATALOG: CatalogEntry[] = [
	// --- providers pi already ships: inject the key, keep pi's model list ----
	{ id: "groq", name: "Groq", slug: "groq", builtin: true, keyUrl: "https://console.groq.com/keys" },
	{
		id: "nvidia",
		name: "NVIDIA NIM",
		slug: "nvidia-nim",
		builtin: true,
		keyUrl: "https://build.nvidia.com/settings/api-keys",
		note: "phone verification; /logout nvidia if auth.json holds a key",
	},
	{ id: "google", name: "Google AI Studio", slug: "gemini", builtin: true, keyUrl: "https://aistudio.google.com/app/apikey" },
	{
		id: "deepseek",
		name: "DeepSeek",
		slug: "deepseek",
		builtin: true,
		vaultItem: "Deepseek API Key",
		keyUrl: "https://platform.deepseek.com/api_keys",
	},
	{
		id: "openrouter",
		name: "OpenRouter",
		slug: "openrouter",
		builtin: true,
		vaultItem: "Openrouter API Key",
		keyUrl: "https://openrouter.ai/workspaces/default/keys",
		note: "two vault items share this name; the one in the Axon folder wins",
	},
	{
		id: "huggingface",
		name: "Hugging Face",
		slug: "huggingface",
		builtin: true,
		keyUrl: "https://huggingface.co/settings/tokens",
		note: "the vault's huggingface.co item is the site login, not a token; create inference-huggingface-api-key",
	},
	{
		id: "kimi-coding",
		name: "Kimi For Coding",
		slug: "kimi",
		builtin: true,
		vaultItem: "Kimi K2 API Token",
		keyUrl: "https://platform.moonshot.ai/console/api-keys",
	},
	{ id: "cerebras", name: "Cerebras", slug: "cerebras", builtin: true, keyUrl: "https://cloud.cerebras.ai/" },
	{ id: "mistral", name: "Mistral AI", slug: "mistral", builtin: true, keyUrl: "https://console.mistral.ai/api-keys" },
	{ id: "xai", name: "xAI", slug: "xai", builtin: true, keyUrl: "https://console.x.ai" },
	{ id: "zai", name: "Z.ai", slug: "zai", builtin: true, keyUrl: "https://open.bigmodel.cn/usercenter/apikeys" },
	{ id: "minimax", name: "MiniMax", slug: "minimax", builtin: true, keyUrl: "https://platform.minimaxi.com/" },
	{ id: "together", name: "Together AI", slug: "together", builtin: true, keyUrl: "https://api.together.ai/settings/api-keys" },
	{ id: "fireworks", name: "Fireworks", slug: "fireworks", builtin: true, keyUrl: "https://fireworks.ai/account/api-keys" },

	// --- providers pi does not ship -----------------------------------------
	{
		id: "ollama-cloud",
		name: "Ollama Cloud",
		slug: "ollama-cloud",
		baseUrl: "https://ollama.com/v1",
		api: "openai-completions",
		compat: OLLAMA_COMPAT,
		keyUrl: "https://ollama.com/settings/keys",
		note: "session/weekly limits rather than RPM; /v1/models is public",
		discover: { url: "https://ollama.com/v1/models", auth: false },
		models: [
			m("deepseek-v4-pro", 1_000_000, true),
			m("deepseek-v4-flash", 1_000_000, true),
			m("deepseek-v4.1-flash", 1_000_000, true),
			m("minimax-m3", 512_000, true),
			m("minimax-m2.7", 512_000, true),
			m("kimi-k3", 262_144, true),
			m("kimi-k2.6", 262_144, true),
			m("kimi-k2.7-code", 262_144, true),
			m("glm-5.3", 202_752, true),
			m("glm-5.2", 202_752, true),
			m("glm-5.1", 202_752, true),
			m("glm-5.3-flash", 202_752, true),
			m("qwen3.5:397b", 262_144, true),
			m("nemotron-3-ultra", 1_000_000, true),
			m("nemotron-3-super", 262_144, true),
			m("nemotron-3-nano:30b", 131_072, true),
			m("gpt-oss:120b", 131_072, true),
			m("gpt-oss:20b", 131_072, true),
			m("mistral-large-3:675b", 262_144, false),
			m("gemma4:31b", 131_072, false, { image: true }),
			// Ollama clamps num_predict rather than rejecting it, so the whole entry
			// can carry the output budget a coding agent actually wants.
		].map((model) => ({ ...model, maxTokens: 32768 })),
	},
	{
		id: "llm7",
		name: "LLM7.io",
		slug: "llm7",
		baseUrl: "https://api.llm7.io/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://token.llm7.io",
		discover: { url: "https://api.llm7.io/v1/models" },
		models: [m("gpt-oss-20b", 131_072, true), m("minimax-m2.7", 184_320, true), m("mistral-Nemo-Instruct-2407", 131_072, false)],
	},
	{
		id: "modelscope",
		name: "ModelScope",
		slug: "modelscope",
		baseUrl: "https://api-inference.modelscope.cn/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://modelscope.cn/my/myaccesstoken",
		discover: { url: "https://api-inference.modelscope.cn/v1/models" },
		models: [m("MiniMax/MiniMax-M2.5", 208_896, true), m("Qwen/Qwen3.5-35B-A3B", 262_144, true), m("Qwen/Qwen3.5-27B", 262_144, true)],
	},
	{
		id: "sambanova",
		name: "SambaNova",
		slug: "sambanova",
		baseUrl: "https://api.sambanova.ai/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://cloud.sambanova.ai/apis",
		discover: { url: "https://api.sambanova.ai/v1/models" },
		models: [m("DeepSeek-V3.1", 131_072, true), m("MiniMax-M2.7", 131_072, true)],
	},
	{
		id: "nebius",
		name: "Nebius",
		slug: "nebius",
		baseUrl: "https://api.studio.nebius.com/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://studio.nebius.com/settings/api-keys",
		discover: { url: "https://api.studio.nebius.com/v1/models" },
		models: [m("Qwen/Qwen3-235B-A22B", 131_072, true)],
	},
	{
		id: "nscale",
		name: "Nscale",
		slug: "nscale",
		baseUrl: "https://inference.api.nscale.com/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://console.nscale.com/",
		discover: { url: "https://inference.api.nscale.com/v1/models" },
		models: [m("meta-llama/Llama-3.3-70B-Instruct", 131_072, false), m("deepseek-ai/DeepSeek-R1-Distill-Llama-70B", 131_072, true)],
	},
	{
		id: "siliconflow",
		name: "SiliconFlow",
		slug: "siliconflow",
		baseUrl: "https://api.siliconflow.cn/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://cloud.siliconflow.cn/account/ak",
		discover: { url: "https://api.siliconflow.cn/v1/models" },
		models: [m("deepseek-ai/DeepSeek-R1-Distill-Qwen-7B", 131_072, true)],
	},
	{
		id: "chutes",
		name: "Chutes.ai",
		slug: "chutes",
		baseUrl: "https://api.chutes.ai/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://chutes.ai/",
		discover: { url: "https://api.chutes.ai/v1/models" },
		models: [m("deepseek-ai/DeepSeek-R1", 131_072, true), m("meta-llama/Meta-Llama-3.1-70B-Instruct", 131_072, false)],
	},
	{
		id: "ovhcloud",
		name: "OVHcloud AI Endpoints",
		slug: "ovhcloud",
		baseUrl: "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://www.ovhcloud.com/en/public-cloud/ai-endpoints/catalog/",
		discover: { url: "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/models" },
		models: [m("qwen3.5-397b-a17b", 131_072, true), m("meta-llama-3_3-70b-instruct", 131_072, false), m("qwen3.6-27b", 131_072, true)],
	},
	{
		id: "glhf",
		name: "Glhf.chat",
		slug: "glhf",
		baseUrl: "https://glhf.chat/api/openai/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://glhf.chat/",
		discover: { url: "https://glhf.chat/api/openai/v1/models" },
		models: [m("meta-llama/Meta-Llama-3.1-70B-Instruct", 131_072, false), m("mistralai/Mixtral-8x7B-Instruct-v0.1", 32_768, false)],
	},
	{
		id: "dashscope",
		name: "Alibaba Model Studio",
		slug: "dashscope",
		baseUrl: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://bailian.console.alibabacloud.com/?apiKey=1",
		discover: { url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models" },
		models: [m("qwen3-max", 131_072, true), m("qwen3-plus", 1_000_000, true)],
	},
	{
		id: "kilocode",
		name: "Kilo Code",
		slug: "kilocode",
		baseUrl: "https://api.kilo.ai/api/gateway",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://app.kilo.ai/profile",
		note: "OpenRouter-shaped gateway; the :free suffix is part of the model id",
		models: [m("nvidia/nemotron-3-ultra-550b-a55b:free", 1_000_000, true), m("stepfun/step-3.7-flash:free", 262_144, true)],
	},
	{
		id: "aionlabs",
		name: "Aion Labs",
		slug: "aionlabs",
		baseUrl: "https://api.aionlabs.ai/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://www.aionlabs.ai/app/api-keys/",
		discover: { url: "https://api.aionlabs.ai/v1/models" },
		models: [m("aion-labs/aion-2.0", 131_072, false), m("aion-labs/aion-3.0", 131_072, false)],
	},
	{
		id: "agnes",
		name: "Agnes AI",
		slug: "agnes",
		baseUrl: "https://apihub.agnes-ai.com/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://platform.agnes-ai.com/settings/apiKeys",
		discover: { url: "https://apihub.agnes-ai.com/v1/models" },
		models: [m("agnes-2.0-flash", 262_144, false), m("agnes-1.5-flash", 262_144, false)],
	},
	{
		id: "ai21",
		name: "AI21 Labs",
		slug: "ai21",
		baseUrl: "https://api.ai21.com/studio/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://studio.ai21.com/account/api-key",
		models: [m("jamba-large-1.7", 262_144, false), m("jamba-mini-2", 262_144, false)],
	},
	{
		id: "github-models",
		name: "GitHub Models",
		slug: "github-models",
		baseUrl: "https://models.github.ai/inference",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://github.com/settings/tokens",
		note: "needs a PAT with models:read; the ambient GitHub token is deliberately unused",
		discover: { url: "https://models.github.ai/inference/models" },
		models: [m("openai/gpt-4.1", 131_072, false), m("Phi-4", 131_072, false), m("Mistral-large-2411", 131_072, false), m("AI21-Jamba-1.5-Large", 262_144, false)],
	},
	{
		id: "cohere",
		name: "Cohere",
		slug: "cohere",
		baseUrl: "https://api.cohere.ai/compatibility/v1",
		api: "openai-completions",
		compat: GATEWAY_COMPAT,
		keyUrl: "https://dashboard.cohere.com/api-keys",
		note: "the OpenAI-compatibility endpoint, not Cohere's native v2 API",
		discover: { url: "https://api.cohere.ai/compatibility/v1/models" },
		models: [
			// Cohere's compatibility endpoint caps output at 8192 and rejects more.
			m("command-a-03-2025", 262_144, false, { maxTokens: 8192 }),
			m("command-r-plus-08-2024", 131_072, false, { maxTokens: 8192 }),
		],
	},
];

const CONVENTIONAL_ITEM = (slug: string) => `inference-${slug}-api-key`;
const vaultItemFor = (entry: CatalogEntry) => entry.vaultItem ?? CONVENTIONAL_ITEM(entry.slug);
/** `slug` or `slug=Exact Item Name`, the form bw-key.mjs --manifest takes. */
const manifestSpec = (entry: CatalogEntry) =>
	vaultItemFor(entry) === CONVENTIONAL_ITEM(entry.slug) ? entry.slug : `${entry.slug}=${vaultItemFor(entry)}`;

/**
 * For models discovery finds that this file has never heard of. Reasoning is
 * guessed from the family, because a wrong `true` is harmless while a wrong
 * `false` silently costs the model its thinking mode; an unrecognised model
 * still works, it just gets the conservative context window.
 */
const REASONING_HINT =
	/(^|[/:_-])(r1|reasoner|reasoning|thinking|think|deepseek-v[34]|glm-[4-9]|kimi-k|qwen3|minimax-m|nemotron-3|gpt-oss|magistral|jamba-large|grok-[4-9]|inkling|laguna)/i;

/**
 * pi runs an `apiKey` value through `sh -c`, so a spec carrying a name with
 * spaces has to be quoted. Embedded single quotes are closed, escaped and
 * reopened rather than rejected, because a vault item name is the user's to
 * choose.
 */
function shellQuote(value: string): string {
	return /^[A-Za-z0-9._=/@:-]+$/.test(value) ? value : `'${value.replace(/'/g, "'\\''")}'`;
}

function inferredModel(id: string): ModelDef {
	return m(id, 131_072, REASONING_HINT.test(id));
}

// ---------------------------------------------------------------------------
// bw-key plumbing
// ---------------------------------------------------------------------------

const EXT_DIR = join(homedir(), ".pi", "agent", "extensions", "inference-keys");
const CACHE_DIR = process.env.PI_BW_CACHE_DIR || join(process.env.XDG_CACHE_HOME || join(homedir(), ".cache"), "pi-inference-keys");
const MANIFEST_FILE = join(CACHE_DIR, "manifest.json");
const LAST_ERROR_FILE = join(CACHE_DIR, "last-error.json");
/** pi discards stderr from apiKey commands and only reports a bare command
 * string, hence the breadcrumb file and the /bw-status diagnostics. */
const KEY_TIMEOUT_MS = 9000;
const MANIFEST_TIMEOUT_MS = 60_000;
const MANIFEST_FRESH_MS = 10 * 60_000;

/**
 * Where bw-key.mjs lives. Resolved from this file's own URL so the extension
 * keeps working if it is moved — into a checkout, say — with the install-path
 * guess kept as a fallback for a host that does not expose import.meta.
 */
function helperPath(): string | undefined {
	let ownDir: string | undefined;
	try {
		ownDir = dirname(fileURLToPath(import.meta.url));
	} catch {
		ownDir = undefined;
	}
	const candidates = [process.env.PI_BW_KEY, ownDir && join(ownDir, "bw-key.mjs"), join(EXT_DIR, "bw-key.mjs")].filter(
		Boolean,
	) as string[];
	return candidates.find((candidate) => existsSync(candidate));
}

function runHelper(helper: string, args: string[], timeout: number): Promise<string> {
	return new Promise((resolve, reject) => {
		execFile(helper, args, { timeout, maxBuffer: 32 * 1024 * 1024 }, (error, stdout) => {
			if (error) reject(error);
			else resolve(stdout);
		});
	});
}

interface ManifestItem {
	item: string;
	exists: boolean;
	ambiguous?: boolean;
	id?: string;
}
interface Manifest {
	checkedAt: string;
	vault: string;
	error?: string;
	/** Set when a refresh attempt failed and this is the previously cached answer. */
	refreshFailed?: string;
	items: Record<string, ManifestItem>;
}

function readManifest(): Manifest | undefined {
	try {
		return JSON.parse(readFileSync(MANIFEST_FILE, "utf8")) as Manifest;
	} catch {
		return undefined;
	}
}

function writeManifest(manifest: Manifest): void {
	try {
		mkdirSync(CACHE_DIR, { recursive: true, mode: 0o700 });
		writeFileSync(MANIFEST_FILE, JSON.stringify(manifest, null, 2), { mode: 0o600 });
	} catch {}
}

function readLastError(): { slug: string; message: string; at: string } | undefined {
	try {
		return JSON.parse(readFileSync(LAST_ERROR_FILE, "utf8"));
	} catch {
		return undefined;
	}
}

/**
 * Ask bw-key which catalog items exist.
 *
 * Only a listing that was actually read replaces the cache. bw-key exits
 * non-zero when the vault could not be read, and that failure keeps the previous
 * inventory instead: caching an empty answer would present "this vault has no
 * keys" and empty the model picker until the cache expired.
 */
async function loadManifest(helper: string | undefined, force: boolean): Promise<Manifest | undefined> {
	if (!helper) return undefined;
	if (!force) {
		const cached = readManifest();
		if (cached && Date.now() - Date.parse(cached.checkedAt) < MANIFEST_FRESH_MS) return cached;
	}
	try {
		const stdout = await runHelper(helper, ["--manifest", ...CATALOG.map(manifestSpec)], MANIFEST_TIMEOUT_MS);
		const manifest = JSON.parse(stdout) as Manifest;
		if (manifest.vault !== "unlocked") throw new Error(manifest.error ?? `vault is ${manifest.vault}`);
		writeManifest(manifest);
		return manifest;
	} catch (error) {
		const reason = error instanceof Error ? error.message : String(error);
		const cached = readManifest();
		if (cached) return { ...cached, refreshFailed: reason };
		return { checkedAt: new Date().toISOString(), vault: "unavailable", error: reason, items: {} };
	}
}

// ---------------------------------------------------------------------------
// Model discovery
// ---------------------------------------------------------------------------

/** OpenAI returns {data:[{id}]}, Ollama's own API {models:[{name}]}; accept both. */
function extractIds(payload: unknown): string[] {
	const list = Array.isArray(payload)
		? payload
		: ((payload as { data?: unknown[]; models?: unknown[] })?.data ?? (payload as { models?: unknown[] })?.models ?? []);
	if (!Array.isArray(list)) return [];
	const ids = list
		.map((raw) => {
			if (typeof raw === "string") return raw;
			const entry = raw as { id?: string; name?: string };
			return entry.id ?? entry.name;
		})
		.filter((id): id is string => typeof id === "string" && id.length > 0);
	return [...new Set(ids)];
}

/**
 * Catalog model facts in pi's model shape.
 *
 * `provider` is not decoration here: this same function builds the payload
 * persisted for a discovered catalog, and pi reads those records back per
 * provider. Without it a stored model can never be matched to its provider, so
 * discovery would restart from the curated handful on every launch.
 */
function toProviderModel(entry: ServedEntry, model: ModelDef) {
	return {
		provider: entry.id,
		id: model.id,
		name: model.name,
		api: entry.api,
		baseUrl: entry.baseUrl,
		reasoning: model.reasoning,
		input: model.input,
		contextWindow: model.contextWindow,
		maxTokens: model.maxTokens,
		cost: model.cost,
		compat: entry.compat,
	};
}

/**
 * Metadata for a discovered id. Ollama re-tags cloud models by build date
 * (`deepseek-v4-pro:0813`), so a curated `deepseek-v4-pro` is treated as the
 * metadata for every tag of that model instead of being discarded as stale —
 * otherwise a re-tag silently costs the model its real context window.
 */
function curatedFor(known: Map<string, ModelDef>, id: string): ModelDef | undefined {
	const exact = known.get(id);
	if (exact) return exact;
	let best: ModelDef | undefined;
	for (const key of known.keys()) {
		if (!id.startsWith(`${key}:`)) continue;
		if (!best || key.length > best.id.length) best = known.get(key);
	}
	return best;
}

/**
 * Curated entries supply metadata, the endpoint supplies the id set. The
 * endpoint wins on membership so a retired model disappears instead of being
 * resurrected from this file; curated metadata wins on facts.
 */
function mergeModels(entry: CatalogEntry, ids: string[], remembered: ModelDef[]): ModelDef[] {
	const known = new Map<string, ModelDef>();
	for (const model of [...(entry.models ?? []), ...remembered]) {
		if (!known.has(model.id)) known.set(model.id, model);
	}
	const ordered = [...(entry.models ?? []).map((model) => model.id), ...ids];
	return [...new Set(ordered)]
		.filter((id) => ids.includes(id))
		.map((id) => {
			const meta = curatedFor(known, id) ?? inferredModel(id);
			// The endpoint's id is what the provider accepts, so only the facts are borrowed.
			return { ...meta, id, name: id };
		});
}

/**
 * Refresh one provider's model list from its own /models endpoint.
 *
 * pi calls this twice per refresh: first with `allowNetwork: false` to restore
 * cached state, then with network access. A composed provider has to perform
 * that restore itself, because pi only keeps what this function returns —
 * returning nothing on the first call would leave the picker showing just the
 * curated models whenever the vault is locked or the machine is offline.
 */
function discover(entry: ServedEntry, helper: string) {
	return async (context: RefreshModelsContext) => {
		const remembered: ModelDef[] = (context.stored?.models ?? [])
			// pi reads these records per provider, so this only has to reject one
			// that names a different provider. A record written before the catalog
			// started stamping `provider` is still this provider's own.
			.filter((model) => model.provider === undefined || model.provider === entry.id)
			// A stored record is an `AnyModel`, and only a chat model has `reasoning`
			// and `contextWindow` at all. pi's `isModelType(model, "chat")` says the same
			// thing, and is deliberately not imported: every extension in this repository
			// imports pi's packages as types only, and this one sits on the credential
			// path, where a load-time import is a new way for provider auth to fail.
			// Excluding the two non-chat discriminants narrows identically and keeps
			// records written before the `type` field existed, which are chat.
			.filter((model) => model.type !== "image" && model.type !== "classifier")
			.map((model) => ({
				id: model.id,
				name: model.name ?? model.id,
				reasoning: model.reasoning ?? false,
				input: model.input ?? ["text"],
				contextWindow: model.contextWindow ?? 131_072,
				// Deliberately not the stored cap: a stored record was written by whichever
				// catalog was then current, and an output cap above the provider's limit
				// fails the whole request. Caps always follow this file.
				maxTokens: DEFAULT_MAX_TOKENS,
				cost: model.cost ?? { ...FREE },
			}));
		const rememberedIds = remembered.map((model) => model.id);
		/**
		 * What to answer when the endpoint cannot be consulted. With nothing
		 * remembered this must be the curated baseline: pi replaces the provider's
		 * model list with whatever this function returns, so an empty array here
		 * registers the provider with no models at all until pi is restarted.
		 */
		const restored = () =>
			rememberedIds.length > 0
				? mergeModels(entry, rememberedIds, remembered).map((model) => toProviderModel(entry, model))
				: (entry.models ?? []).map((model) => toProviderModel(entry, model));

		if (!context.allowNetwork || context.signal.aborted) return restored();

		const headers: Record<string, string> = {};
		if (entry.discover?.auth !== false) {
			const key = await runHelper(helper, [manifestSpec(entry)], KEY_TIMEOUT_MS)
				.then((value) => value.trim())
				.catch(() => "");
			if (key) headers.Authorization = `Bearer ${key}`;
		}

		try {
			const response = await fetch(entry.discover!.url, { headers, signal: context.signal });
			if (!response.ok) throw new Error(`${entry.name} model listing returned HTTP ${response.status}`);
			const ids = extractIds(await response.json());
			if (ids.length === 0) throw new Error(`${entry.name} model listing was empty`);
			const models = mergeModels(entry, ids, remembered);
			await context.publish({
				persist: { models: models.map((model) => toProviderModel(entry, model)), checkedAt: Date.now() },
			});
			return models.map((model) => toProviderModel(entry, model));
		} catch (error) {
			// Keep the last good list instead of collapsing to the curated handful;
			// only a provider that has never succeeded is allowed to fail loudly.
			if (remembered.length === 0 || context.signal.aborted) throw error;
			return restored();
		}
	};
}

// ---------------------------------------------------------------------------
// Extension
// ---------------------------------------------------------------------------

export default function (pi: ExtensionAPI) {
	const helper = helperPath();
	const state = { registry: new Set<string>(), failures: new Map<string, string>() };

	/**
	 * One malformed entry must not cost the user every other provider, so a
	 * registration failure is recorded and shown by /bw-status instead of
	 * aborting the catalog. The id is only marked registered after pi accepts it.
	 */
	function register(entry: CatalogEntry) {
		if (state.registry.has(entry.id)) return;
		const apiKey = `!${shellQuote(helper!)} ${shellQuote(manifestSpec(entry))}`;
		try {
			if (entry.builtin) {
				// Auth only. Supplying `models` here would replace pi's curated catalog
				// for this provider, which is the one thing to avoid.
				pi.registerProvider(entry.id, { apiKey });
			} else {
				const backend = servedEntry(entry);
				if (!backend) throw new Error("a provider pi does not ship needs a baseUrl and an api");
				pi.registerProvider(entry.id, {
					name: entry.name,
					baseUrl: backend.baseUrl,
					api: backend.api,
					apiKey,
					models: (entry.models ?? []).map((model) => toProviderModel(backend, model)),
					refreshModels: entry.discover ? discover(backend, helper!) : undefined,
				});
			}
			state.registry.add(entry.id);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			state.failures.set(entry.id, message);
			// Written as well as remembered: pi's model listing runs without a UI, so a
			// notify alone would leave a silent gap in the catalog.
			try {
				mkdirSync(CACHE_DIR, { recursive: true, mode: 0o700 });
				writeFileSync(
					join(CACHE_DIR, "registration-errors.json"),
					JSON.stringify(Object.fromEntries(state.failures), null, 2),
					{ mode: 0o600 },
				);
			} catch {}
		}
	}

	if (!helper) {
		pi.on("session_start", (_event, ctx) => {
			ctx.ui.notify(`inference-keys: bw-key.mjs not found (looked in ${EXT_DIR}); set PI_BW_KEY`, "error");
		});
		return;
	}

	/** Register every catalog entry the inventory says exists; returns the registry size. */
	function registerFrom(manifest: Manifest | undefined): number {
		for (const entry of CATALOG) if (manifest?.items[entry.slug]?.exists) register(entry);
		return state.registry.size;
	}

	// Startup must never wait on the vault. This module is evaluated before pi's
	// TUI exists, and one read spawns `bw` about five times — 8.3 s measured on
	// 2026-09-13, including a sync to a self-hosted server over Tailscale. So the
	// inventory on disk is registered synchronously (one file read) and the vault
	// is consulted afterwards, off the startup path. The price of a stale
	// inventory is a provider that appears a few seconds late, or one that has
	// since lost its key and fails when used; session_start reports the arrival.
	registerFrom(readManifest());

	/**
	 * One refresh at a time: a session that starts while a read is in flight joins
	 * it instead of stacking a second `bw`. A fresh cache returns it without
	 * spawning anything, which is what MANIFEST_FRESH_MS is for.
	 */
	let refreshing: Promise<Manifest | undefined> | undefined;
	function refresh(): Promise<Manifest | undefined> {
		refreshing ??= loadManifest(helper!, false).finally(() => {
			refreshing = undefined;
		});
		return refreshing;
	}

	// Started here, not in session_start, so the read overlaps the rest of pi's
	// startup instead of beginning after the UI is up.
	void refresh();

	pi.on("session_start", (_event, ctx) => {
		const before = state.registry.size;
		// Joins the read above when it is still in flight; never awaited, because
		// session_start gates pi's first prompt. The notifications land when it does.
		void refresh().then((manifest) => {
			registerFrom(manifest);
			try {
				if (state.registry.size > before) {
					ctx.ui.notify(`inference-keys: ${state.registry.size - before} more provider(s) found in the vault`, "info");
				}
				if (manifest?.refreshFailed) {
					ctx.ui.notify(
						`inference-keys: vault unreadable (${manifest.refreshFailed}); still using the inventory from ${new Date(manifest.checkedAt).toLocaleString()}`,
						"warning",
					);
				}
				for (const [id, message] of state.failures) ctx.ui.notify(`inference-keys: ${id} not registered — ${message}`, "error");
				if (state.registry.size === 0 && manifest?.error) ctx.ui.notify(`inference-keys: ${manifest.error}`, "warning");
			} catch {
				// The session can be gone by the time a read lands (reload, switch), and
				// these are diagnostics — never worth failing the refresh over.
			}
		});
	});

	pi.registerCommand("bw-status", {
		description: "Bitwarden-backed providers: vault state, resolvable keys, missing items",
		handler: async (_args, ctx) => {
			let status: Record<string, unknown> = {};
			try {
				status = JSON.parse(await runHelper(helper, ["--status"], KEY_TIMEOUT_MS));
			} catch (error) {
				status = { error: error instanceof Error ? error.message : String(error) };
			}
			const manifest = readManifest();
			const vault = (status.vault ?? {}) as { status?: string; serverUrl?: string };
			const lastError = readLastError();
			const lines: string[] = [];
			lines.push(`vault: ${vault.status ?? "unknown"}${vault.serverUrl ? `  (${vault.serverUrl})` : ""}`);
			lines.push(`session: ${status.sessionUsable ? "usable" : "none"}  from ${(status.sessionCandidates as string[] | undefined)?.join(", ") || "nothing"}`);
			lines.push(`key cache: ${Number(status.ttlSeconds) > 0 ? `${status.ttlSeconds}s TTL in ${status.cacheDir}` : "disabled (PI_BW_TTL=0)"}`);
			lines.push(`items listed: ${manifest ? new Date(manifest.checkedAt).toLocaleString() : "never"}`);
			lines.push("");
			for (const entry of CATALOG) {
				const item = manifest?.items[entry.slug];
				const registered = state.registry.has(entry.id);
				const mark = registered ? "ready " : item?.exists ? "found " : "missing";
				const extra = item?.ambiguous ? "  (ambiguous name)" : "";
				lines.push(`${mark}  ${entry.id.padEnd(16)} ${vaultItemFor(entry)}${extra}`);
			}
			if (lastError) lines.push("", `last failure: ${lastError.slug} at ${new Date(lastError.at).toLocaleTimeString()} — ${lastError.message}`);
			for (const [id, message] of state.failures) lines.push(`registration failed: ${id} — ${message}`);
			pi.sendMessage({ customType: "bw-status", content: lines.join("\n"), display: true });
		},
	});

	pi.registerCommand("bw-unlock", {
		description: "Unlock the Bitwarden vault via the Axon helper, then re-read the key inventory",
		handler: async (_args, ctx) => {
			ctx.ui.notify("Unlocking the vault — authorize the keychain prompt", "info");
			try {
				await runHelper(helper, ["--forget"], 5000);
				const out = await runHelper(helper, ["--unlock"], 120_000);
				const status = JSON.parse(out) as { status?: string };
				ctx.ui.notify(`Vault ${status.status ?? "?"}`, status.status === "unlocked" ? "info" : "warning");
			} catch (error) {
				ctx.ui.notify(`Unlock failed: ${error instanceof Error ? error.message : String(error)}`, "error");
				return;
			}
			const manifest = await loadManifest(helper, true);
			const added: string[] = [];
			for (const entry of CATALOG) {
				if (manifest?.items[entry.slug]?.exists && !state.registry.has(entry.id)) {
					register(entry);
					added.push(entry.id);
				}
			}
			ctx.ui.notify(added.length > 0 ? `Newly enabled: ${added.join(", ")}` : "No new vault items found", "info");
		},
	});

	pi.registerCommand("bw-refresh", {
		description: "Drop cached keys, re-read the vault inventory, and re-discover provider models",
		handler: async (_args, ctx) => {
			ctx.ui.notify("Re-reading the vault and refreshing model lists", "info");
			await runHelper(helper, ["--forget"], 5000).catch(() => "");
			const manifest = await loadManifest(helper, true);
			const added: string[] = [];
			for (const entry of CATALOG) {
				if (manifest?.items[entry.slug]?.exists && !state.registry.has(entry.id)) {
					register(entry);
					added.push(entry.id);
				}
			}
			try {
				const result = await ctx.modelRegistry.refresh({ force: true });
				const failed = [...result.errors.keys()];
				ctx.ui.notify(
					`Vault ${manifest?.vault ?? "?"}${added.length > 0 ? `, newly enabled ${added.join(", ")}` : ""}` +
						(failed.length > 0 ? `; model refresh failed for ${failed.join(", ")}` : ""),
					failed.length > 0 ? "warning" : "info",
				);
			} catch (error) {
				ctx.ui.notify(`Model refresh failed: ${error instanceof Error ? error.message : String(error)}`, "warning");
			}
		},
	});
}
