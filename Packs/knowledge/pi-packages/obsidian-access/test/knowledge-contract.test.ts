import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadKnowledgeContract, loadKnowledgeSettings } from "../src/knowledge-contract.ts";

const CONTRACT = "Meta/Vault-Contract.md";

function fixture(body = `version = 1\nprofile = "work"\ncontract_path = "${CONTRACT}"\n`) {
	const root = mkdtempSync(join(tmpdir(), "sjel-knowledge-contract-"));
	mkdirSync(join(root, "config"));
	const config = join(root, "config", "knowledge.toml");
	writeFileSync(config, body);
	return { root, config };
}

const noRead = async (): Promise<string> => {
	throw new Error("the contract must not be read");
};

describe("configured vault contract", () => {
	test("reads the configured contract path and returns a stable digest", async () => {
		const { config } = fixture();
		const readContract = async (path: string) => {
			expect(path).toBe(CONTRACT);
			return "# Contract\n\nRules.\n";
		};
		const first = await loadKnowledgeContract(readContract, config);
		const second = await loadKnowledgeContract(readContract, config);
		expect(first).toMatchObject({ profile: "work", path: CONTRACT, content: "# Contract\n\nRules.\n" });
		expect(first?.digest).toHaveLength(64);
		expect(second?.digest).toBe(first?.digest);
	});

	test("is optional when knowledge.toml sets no contract_path", async () => {
		const { config } = fixture('version = 1\nprofile = "work"\n');
		expect(loadKnowledgeSettings(config)).toEqual({ profile: "work", contractPath: null });
		await expect(loadKnowledgeContract(noRead, config)).resolves.toBeNull();
	});

	test("is optional when knowledge.toml is absent", async () => {
		const { root } = fixture();
		const missing = join(root, "config", "absent.toml");
		expect(loadKnowledgeSettings(missing)).toEqual({ profile: null, contractPath: null });
		await expect(loadKnowledgeContract(noRead, missing)).resolves.toBeNull();
	});

	test("fails closed when the configured contract note is missing", async () => {
		const { config } = fixture();
		const readContract = async () => {
			throw new Error("Obsidian object not found");
		};
		await expect(loadKnowledgeContract(readContract, config)).rejects.toThrow(
			`${CONTRACT}: configured vault contract could not be read: Obsidian object not found`,
		);
	});

	test("refuses an empty configured contract", async () => {
		const { config } = fixture();
		await expect(loadKnowledgeContract(async () => "\n", config)).rejects.toThrow("contract is empty");
	});

	test("refuses an oversized configured contract", async () => {
		const { config } = fixture();
		await expect(loadKnowledgeContract(async () => "x".repeat(50_001), config)).rejects.toThrow("50000-byte limit");
	});

	test("rejects a contract_path outside the vault", () => {
		for (const value of ["/etc/contract.md", "../Contract.md", "Meta/Contract.txt", ""]) {
			const { config } = fixture(`version = 1\ncontract_path = "${value}"\n`);
			expect(() => loadKnowledgeSettings(config)).toThrow("contract_path");
		}
	});
});
