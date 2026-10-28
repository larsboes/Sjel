import { describe, expect, test } from "bun:test";
import { SessionClearance } from "../src/clearance.ts";
import type { AccessConfig, Classification } from "../src/contracts.ts";

const config: AccessConfig = {
	version: 1,
	defaultMaxClass: "c0",
	defaultAccess: ["standard"],
	paths: [],
	models: [
		{ pattern: "local/*", maxClass: "c2", access: ["standard", "internal", "confidential"], processing: "local", c2Approved: true },
		{ pattern: "cloud/*", maxClass: "c1", access: ["standard"], processing: "external", c2Approved: false },
	],
	transport: "fetch",
	baseUrl: "http://localhost",
	maxResponseBytes: 50_000,
	requestTimeoutSeconds: 120,
	maxCollectionItems: 50,
	classifierPath: null,
	egressTools: ["web_search", "fetch_content"],
	auditPath: null,
};

function classified(dataClass: Classification["dataClass"], agentAccess: Classification["agentAccess"]): Classification {
	return { dataClass, agentAccess, rationale: "fixture", method: "frontmatter", valid: true };
}

describe("session clearance", () => {
	test("only moves upward across admitted reads", () => {
		const clearance = new SessionClearance();
		expect(clearance.raise(classified("c1", "standard"))).toBe(true);
		expect(clearance.raise(classified("c0", "standard"))).toBe(false);
		expect(clearance.raise(classified("c1", "internal"))).toBe(true);
		expect(clearance.raise(classified("c2", "standard"))).toBe(true);
		expect(clearance.value).toEqual({ dataClass: "c2", agentAccess: "internal" });
	});

	test("cannot record C3 because C3 is never admitted", () => {
		const clearance = new SessionClearance();
		expect(clearance.raise(classified("c3", "confidential"))).toBe(false);
		expect(clearance.value).toBeNull();
	});

	test("blocks a lower-clearance model after protected content entered the session", () => {
		const clearance = new SessionClearance();
		clearance.raise(classified("c2", "internal"));
		expect(clearance.decide(config, { provider: "local", id: "qwen" })?.allowed).toBe(true);
		expect(clearance.decide(config, { provider: "cloud", id: "reviewed" })?.allowed).toBe(false);
	});

	test("restores only valid persisted entries", () => {
		const clearance = new SessionClearance();
		expect(clearance.restore({ dataClass: "c2", agentAccess: "confidential" })).toBe(true);
		expect(clearance.restore({ dataClass: "c3", agentAccess: "confidential" })).toBe(false);
		expect(clearance.value).toEqual({ dataClass: "c2", agentAccess: "confidential" });
	});
});
