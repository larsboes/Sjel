import { describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AccessConfig, Classification } from "../src/contracts.ts";
import { classifyNote, parseAccessFrontmatter } from "../src/frontmatter.ts";
import { applyPathRules, decideAccess, ruleForModel } from "../src/policy.ts";

const config: AccessConfig = {
	version: 1,
	defaultMaxClass: "c0",
	defaultAccess: ["standard"],
	paths: [
		{ pattern: "**/_Internal/**", access: "internal" },
	],
	models: [
		{ pattern: "local/*", maxClass: "c2", access: ["standard", "internal", "confidential"], processing: "local", c2Approved: true },
		{ pattern: "cloud/reviewed", maxClass: "c1", access: ["standard", "internal"], processing: "external", c2Approved: false },
		{ pattern: "trusted-proxy/*", maxClass: "c2", access: ["standard", "internal", "confidential"], processing: "trusted-service", c2Approved: true },
	],
	transport: "fetch",
	baseUrl: "http://127.0.0.1:27123",
	maxResponseBytes: 50_000,
	requestTimeoutSeconds: 120,
	maxCollectionItems: 50,
	classifierPath: null,
	egressTools: ["web_search", "fetch_content"],
	auditPath: null,
};

function classification(dataClass: Classification["dataClass"], agentAccess: Classification["agentAccess"]): Classification {
	return { dataClass, agentAccess, rationale: "fixture", method: "frontmatter", valid: true };
}

describe("frontmatter", () => {
	test("absence adds no agent restriction but fails closed at C2 until the canonical classifier is wired", () => {
		expect(classifyNote("# Note\n")).toMatchObject({ dataClass: "c2", agentAccess: "standard", valid: true });
	});

	test("uses the canonical classifier contract for folder defaults", () => {
		const directory = mkdtempSync(join(tmpdir(), "sjel-classifier-"));
		const executable = join(directory, "vault");
		writeFileSync(executable, "#!/bin/sh\nprintf '%s\\n' '{\"value\":\"c1\",\"rationale\":\"canonical fixture\"}'\n");
		chmodSync(executable, 0o700);
		expect(classifyNote("# Note\n", "Journal/Note.md", executable)).toMatchObject({
			dataClass: "c1",
			method: "canonical",
			rationale: "canonical fixture",
		});
	});

	test("reads explicit class and internal access", () => {
		expect(classifyNote("---\nclass: c1\nagent_access: internal\n---\n# Note\n")).toMatchObject({
			dataClass: "c1",
			agentAccess: "internal",
			method: "frontmatter",
			valid: true,
		});
	});

	test("maps the vault's legacy sensitivity boundary without allowing agent_access to weaken it", () => {
		expect(classifyNote("---\nclass: c1\nsensitivity: internal\n---\n")).toMatchObject({ agentAccess: "internal", valid: true });
		expect(classifyNote("---\nclass: c1\nsensitivity: restricted\nagent_access: internal\n---\n")).toMatchObject({ agentAccess: "confidential", valid: true });
		expect(classifyNote("---\nclass: c1\nsensitivity: public\n---\n")).toMatchObject({ agentAccess: "standard", valid: true });
	});

	test("path policy can tighten _Internal without frontmatter weakening it", () => {
		const note = classifyNote("---\nclass: c1\nsensitivity: public\n---\n", "Projects/Example/_Internal/Plan.md");
		expect(applyPathRules(note, "Projects/Example/_Internal/Plan.md", config.paths)).toMatchObject({
			agentAccess: "internal",
			valid: true,
		});
		const base = classifyNote("", "Projects/Example/_Internal/Dashboard.base", null);
		expect(applyPathRules(base, "Projects/Example/_Internal/Dashboard.base", config.paths)).toMatchObject({ agentAccess: "internal" });
		expect(applyPathRules(base, "_Internal/Dashboard.base", config.paths)).toMatchObject({ agentAccess: "internal" });
		expect(applyPathRules(base, "Projects/Example/Internal/Dashboard.base", config.paths)).toMatchObject({ agentAccess: "standard" });
	});

	test("unknown access fails closed", () => {
		expect(classifyNote("---\nclass: c1\nagent_access: private\n---\n")).toMatchObject({
			dataClass: "c3",
			agentAccess: "confidential",
			valid: false,
		});
	});

	test("malformed, duplicate, and unknown policy frontmatter fails closed", () => {
		expect(classifyNote("---\nagent_access: [\n---\n").valid).toBe(false);
		expect(classifyNote("---\nsensitivity: secret-ish\n---\n").valid).toBe(false);
		expect(classifyNote("---\nagent_access: internal\nagent_access: confidential\n---\n").valid).toBe(false);
	});

	test("frontmatter parser ignores body keys", () => {
		expect(parseAccessFrontmatter("# Note\nagent_access: confidential\n")).toEqual({});
	});
});

describe("model policy", () => {
	test("matches exact and wildcard provider/model rules", () => {
		expect(ruleForModel(config, { provider: "local", id: "qwen" }).pattern).toBe("local/*");
		expect(ruleForModel(config, { provider: "cloud", id: "reviewed" }).pattern).toBe("cloud/reviewed");
		expect(ruleForModel(config, { provider: "cloud", id: "other" }).pattern).toBe("<default>");
	});

	test("an explicitly approved trusted service can receive C2", () => {
		expect(decideAccess(config, { provider: "trusted-proxy", id: "reviewed" }, classification("c2", "internal"))).toMatchObject({
			allowed: true,
		});
	});

	test("C3 never enters a prompt", () => {
		expect(decideAccess(config, { provider: "local", id: "qwen" }, classification("c3", "confidential"))).toMatchObject({
			allowed: false,
			reason: "C3 content never enters a prompt.",
		});
	});

	test("class and agent access both have to pass", () => {
		expect(decideAccess(config, { provider: "cloud", id: "reviewed" }, classification("c1", "internal")).allowed).toBe(true);
		expect(decideAccess(config, { provider: "cloud", id: "reviewed" }, classification("c2", "internal")).allowed).toBe(false);
		expect(decideAccess(config, { provider: "cloud", id: "reviewed" }, classification("c1", "confidential")).allowed).toBe(false);
		expect(decideAccess(config, { provider: "local", id: "qwen" }, classification("c2", "confidential")).allowed).toBe(true);
	});

	test("an invalid classification is denied independently of model clearance", () => {
		const invalid = { ...classification("c1", "standard"), valid: false };
		expect(decideAccess(config, { provider: "local", id: "qwen" }, invalid).allowed).toBe(false);
	});
});
