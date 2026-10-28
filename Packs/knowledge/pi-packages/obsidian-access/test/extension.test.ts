import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import obsidianAccess from "../src/index.ts";

const previousOverlay = process.env.SJEL_OVERLAY_ROOT;
const previousAccessConfig = process.env.OBSIDIAN_ACCESS_CONFIG;
const previousApiKey = process.env.OBSIDIAN_API_KEY;
const previousFetch = globalThis.fetch;
afterEach(() => {
	if (previousOverlay === undefined) delete process.env.SJEL_OVERLAY_ROOT;
	else process.env.SJEL_OVERLAY_ROOT = previousOverlay;
	if (previousAccessConfig === undefined) delete process.env.OBSIDIAN_ACCESS_CONFIG;
	else process.env.OBSIDIAN_ACCESS_CONFIG = previousAccessConfig;
	if (previousApiKey === undefined) delete process.env.OBSIDIAN_API_KEY;
	else process.env.OBSIDIAN_API_KEY = previousApiKey;
	globalThis.fetch = previousFetch;
});

function extensionFixture() {
	const tools = new Map<string, any>();
	const entries: Array<{ type: string; data: unknown }> = [];
	const handlers = new Map<string, Function>();
	const pi = {
		registerTool(tool: any) { tools.set(tool.name, tool); },
		on(name: string, handler: Function) { handlers.set(name, handler); },
		appendEntry(type: string, data: unknown) { entries.push({ type, data }); },
		getActiveTools() { return []; },
		setActiveTools() {},
	};
	obsidianAccess(pi as any);
	return { tools, entries, handlers };
}

describe("extension contract gate", () => {
	test("registers low-token validators, Base reads, and property reads", () => {
		const { tools } = extensionFixture();
		for (const name of ["obsidian_note_validate", "obsidian_base_validate", "obsidian_base_read", "obsidian_properties"]) {
			expect(tools.has(name)).toBe(true);
		}
	});

	const MUTATIONS = ["obsidian_create", "obsidian_append", "obsidian_write", "obsidian_patch", "obsidian_move", "obsidian_delete"];
	const CONFIGURED = 'version = 1\nprofile = "work"\ncontract_path = "Meta/Vault-Contract.md"\n';

	function overlay(knowledge: string | null) {
		const root = mkdtempSync(join(tmpdir(), "sjel-extension-contract-"));
		mkdirSync(join(root, "config"));
		if (knowledge !== null) writeFileSync(join(root, "config", "knowledge.toml"), knowledge);
		const accessConfig = join(root, "config", "obsidian-access.toml");
		writeFileSync(accessConfig, 'version = 1\ntransport = "fetch"\n');
		process.env.SJEL_OVERLAY_ROOT = root;
		process.env.OBSIDIAN_ACCESS_CONFIG = accessConfig;
		process.env.OBSIDIAN_API_KEY = "test-key";
		globalThis.fetch = (async () => {
			throw new Error("unexpected network call");
		}) as unknown as typeof fetch;
	}

	function mutate(tools: Map<string, any>, name: string) {
		return tools.get(name).execute("id", { path: "Note.md", content: "body" }, undefined, undefined, {});
	}

	test("with a configured contract, every mutation stops until the contract is loaded", async () => {
		overlay(CONFIGURED);
		const { tools } = extensionFixture();
		for (const name of MUTATIONS) await expect(mutate(tools, name)).rejects.toThrow("Load obsidian_contract");
	});

	test("loading the configured vault contract records its digest and opens the gate", async () => {
		overlay(CONFIGURED);
		globalThis.fetch = (async (input) => {
			expect(String(input)).toContain("/vault/Meta/Vault-Contract.md");
			return new Response("# Contract\n", { status: 200, headers: { "content-type": "text/markdown" } });
		}) as typeof fetch;
		const { tools, entries } = extensionFixture();
		const result = await tools.get("obsidian_contract").execute();
		expect(result.content[0].text).toBe("# Contract\n");
		expect(entries[0]).toMatchObject({ type: "obsidian-contract-loaded", data: { profile: "work", path: "Meta/Vault-Contract.md" } });
		expect((entries[0]?.data as { digest: string }).digest).toHaveLength(64);
		// Past the gate, the next failure is the missing model.
		await expect(mutate(tools, "obsidian_create")).rejects.toThrow("requires an active model");
	});

	test("a configured contract that is missing fails closed", async () => {
		overlay(CONFIGURED);
		globalThis.fetch = (async () => new Response("", { status: 404 })) as unknown as typeof fetch;
		const { tools, entries } = extensionFixture();
		await expect(tools.get("obsidian_contract").execute()).rejects.toThrow("configured vault contract could not be read");
		expect(entries).toHaveLength(0);
		await expect(mutate(tools, "obsidian_create")).rejects.toThrow("Load obsidian_contract");
	});

	for (const [label, knowledge] of [
		["sets no contract_path", 'version = 1\nprofile = "work"\n'],
		["is absent", null],
	] as const) {
		test(`when knowledge.toml ${label}, the contract is optional and mutations are not gated`, async () => {
			overlay(knowledge);
			const { tools, entries } = extensionFixture();
			const result = await tools.get("obsidian_contract").execute();
			expect(result.content[0].text).toContain("No vault contract is configured for this machine");
			expect(result.details).toMatchObject({ configured: false });
			expect(entries).toHaveLength(0);
			for (const name of MUTATIONS) await expect(mutate(tools, name)).rejects.toThrow("requires an active model");
		});
	}

	test("a malformed knowledge.toml keeps mutations gated", async () => {
		overlay('version = 1\ncontract_path = "../outside.md"\n');
		const { tools } = extensionFixture();
		await expect(mutate(tools, "obsidian_create")).rejects.toThrow("contract_path must be a vault-relative .md path");
	});
});
