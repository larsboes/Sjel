import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";

let scratch: string | undefined;
afterEach(() => {
	if (scratch) rmSync(scratch, { recursive: true, force: true });
	scratch = undefined;
});

function config(text: string): string {
	scratch = mkdtempSync(join(tmpdir(), "obsidian-access-test-"));
	const path = join(scratch, "policy.toml");
	writeFileSync(path, text);
	return path;
}

describe("policy configuration", () => {
	test("a missing private policy is C0-only", () => {
		expect(loadConfig("/path/that/does/not/exist")).toMatchObject({
			defaultMaxClass: "c0",
			defaultAccess: ["standard"],
			models: [],
			paths: [],
		});
	});

	test("reads exact model rules", () => {
		const path = config(`
version = 1
default_max_class = "c0"
default_access = ["standard"]
[[models]]
pattern = "local/qwen"
max_class = "c2"
access = ["standard", "internal", "confidential"]
processing = "local"
c2_approved = true
`);
		expect(loadConfig(path).models).toEqual([
			{ pattern: "local/qwen", maxClass: "c2", access: ["standard", "internal", "confidential"], processing: "local", c2Approved: true },
		]);
	});

	test("reads vault-relative path restrictions", () => {
		const path = config(`
version = 1
[[paths]]
pattern = "**/_Internal/**"
access = "internal"
[[paths]]
pattern = "Resources/Bases/Internal.base"
access = "confidential"
`);
		expect(loadConfig(path).paths).toEqual([
			{ pattern: "**/_Internal/**", access: "internal" },
			{ pattern: "Resources/Bases/Internal.base", access: "confidential" },
		]);
	});

	test("refuses invalid path restrictions", () => {
		const path = config('version = 1\n[[paths]]\npattern = "../outside"\naccess = "internal"\n');
		expect(() => loadConfig(path)).toThrow("must stay vault-relative");
	});

	test("refuses C2 without explicit processing-boundary approval", () => {
		const path = config(`
version = 1
[[models]]
pattern = "cloud/*"
max_class = "c2"
access = ["standard"]
`);
		expect(() => loadConfig(path)).toThrow("without explicit c2_approved = true");
	});

	test("refuses any attempt to grant C3", () => {
		const path = config(`
version = 1
[[models]]
pattern = "local/*"
max_class = "c3"
access = ["confidential"]
processing = "local"
c2_approved = true
`);
		expect(() => loadConfig(path)).toThrow("C3 never enters a prompt");
	});
});
